//! Production-oriented font selection and shaping backend for XUI.
//!
//! Responsibilities are separated into platform font management (`font`),
//! Unicode bidi resolution (`bidi`), HarfRust shaping (`shape`), and paragraph
//! line layout (`layout`). Glyph rasterization intentionally remains outside
//! this crate so a native renderer can consume positioned glyph IDs directly.

mod bidi;
mod font;
mod layout;
mod shape;
mod types;

use std::sync::Arc;

use xui_interface::{
    FontDataRef, FontDatabase, FontQuery, GlyphRasterizer, ParagraphLayout, RasterizedGlyph,
    Shaper, TextBackend, TextLayoutInput,
};

pub use layout::{FHardLayout, FParagraphState};
pub use types::{FFontId, FGlyphKey};

/// Fontique font management plus HarfRust shaping and paragraph layout.
pub struct FBackend {
    fonts: font::FontStore,
}

impl FBackend {
    /// Creates a backend and indexes fonts reported by the platform source.
    pub fn new() -> Self {
        let mut backend = Self {
            fonts: font::FontStore::with_system_fonts(true),
        };
        backend.fonts.warm_system_ui();
        backend
    }

    /// Creates an empty backend for applications that only use embedded fonts.
    pub fn without_system_fonts() -> Self {
        Self {
            fonts: font::FontStore::empty(),
        }
    }

    pub fn face_count(&self) -> usize {
        self.fonts.face_count()
    }

    /// Advances Fontique's source-cache generation and removes stale or
    /// failed entries. Materialized faces referenced by XUI remain alive.
    pub fn prune_font_sources(&mut self, max_age: u64) {
        self.fonts.prune_sources(max_age);
    }
}

impl Default for FBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FontDatabase for FBackend {
    type FontId = FFontId;

    fn epoch(&self) -> u64 {
        self.fonts.epoch()
    }

    fn load_system_fonts(&mut self) {
        self.fonts.load_system_fonts();
        self.fonts.warm_system_ui();
    }

    fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> Self::FontId {
        self.fonts.load_font_bytes(bytes)
    }

    fn query(&mut self, query: &FontQuery) -> Option<Self::FontId> {
        self.fonts.query(query)
    }

    fn font_data(&self, id: Self::FontId) -> Option<FontDataRef<'_>> {
        self.fonts.font_data(id)
    }
}

impl Shaper for FBackend {
    type State = FParagraphState;
    type GlyphKey = FGlyphKey;
    type FontId = FFontId;

    fn create_state(&mut self) -> Self::State {
        FParagraphState::default()
    }

    fn layout_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphLayout<Self::FontId, Self::GlyphKey> {
        layout::layout(&mut self.fonts, state, input)
    }
}

impl GlyphRasterizer for FBackend {
    type GlyphKey = FGlyphKey;

    fn rasterize(&mut self, _key: Self::GlyphKey) -> Option<RasterizedGlyph> {
        types::no_rasterized_glyph()
    }
}

impl TextBackend for FBackend {}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{
        ComputedTextStyle, FontFamily, FontStretch, FontStyle, FontWeight, LineHeight,
        OverflowWrap, ParagraphStyle, TextBoxStyle, TextContent, TextLayoutConstraints,
        TextOverflow, TextStyle, WhiteSpace,
    };

    fn input(text: &'static str, width: f32) -> TextLayoutInput {
        TextLayoutInput::new(
            TextContent::from_static(text),
            TextLayoutConstraints::max_width(width),
            ComputedTextStyle::from(TextStyle::default()),
            ParagraphStyle {
                white_space: WhiteSpace::Normal,
                overflow_wrap: OverflowWrap::BreakWord,
                ..ParagraphStyle::default()
            },
            TextBoxStyle::default(),
            0,
        )
    }

    #[test]
    fn production_pipeline_handles_bidi_fallback_and_limits() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        let mut state = backend.create_state();
        let layout =
            backend.layout_paragraph(&mut state, input("English العربية עברית 中文 😀", 180.0));
        assert!(!layout.lines.is_empty());
        assert!(layout.runs.iter().any(|run| run.bidi_level % 2 == 1));
        assert!(layout.runs.iter().any(|run| run.bidi_level % 2 == 0));
        assert!(layout.lines.iter().all(|line| !line.glyph_range.is_empty()));
        assert!(layout.glyphs.iter().any(|glyph| {
            glyph
                .flags
                .contains(xui_interface::GlyphFlags::FALLBACK_FONT)
        }));

        let mut zero_width = input("office", 0.0);
        zero_width.paragraph_style.overflow_wrap = OverflowWrap::Anywhere;
        let narrow = backend.layout_paragraph(&mut state, zero_width);
        assert_eq!(narrow.lines.len(), 6);
        assert!(
            narrow
                .lines
                .iter()
                .all(|line| line.text_range.start.raw < line.text_range.end.raw)
        );
        assert!(
            backend
                .query(&FontQuery {
                    families: vec![FontFamily::System],
                    weight: FontWeight::Bold,
                    style: FontStyle::Italic,
                    stretch: FontStretch::Condensed,
                })
                .is_some()
        );

        let mut request = input("one two three four five six seven", 70.0);
        request.text_box_style.max_lines = Some(1);
        request.text_box_style.overflow = TextOverflow::Ellipsis;
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(&mut state, request);
        assert_eq!(layout.lines.len(), 1);
        assert!(layout.lines[0].ellipsized);
        assert!(
            layout
                .glyphs
                .iter()
                .any(|glyph| { glyph.flags.contains(xui_interface::GlyphFlags::SYNTHETIC) })
        );

        let mut rtl_request = input("אבג דהו זחט", 45.0);
        rtl_request.paragraph_style.white_space = WhiteSpace::NoWrap;
        rtl_request.text_box_style.overflow = TextOverflow::Ellipsis;
        let rtl = backend.layout_paragraph(&mut state, rtl_request);
        assert!(rtl.lines[0].ellipsized);
        assert!(
            rtl.glyphs[0]
                .flags
                .contains(xui_interface::GlyphFlags::SYNTHETIC)
        );
        let synthetic = rtl
            .clusters
            .iter()
            .find(|cluster| cluster.glyph_range.contains(&0))
            .expect("RTL ellipsis cluster");
        assert_eq!(synthetic.text_range.start, rtl.lines[0].text_range.end);
        assert_eq!(synthetic.text_range.end, rtl.lines[0].text_range.end);
    }

    #[test]
    fn line_boxes_fit_the_faces_a_line_actually_uses() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        let mut sized = |line_height: LineHeight| {
            let mut request = input("中文 Agjy", 400.0);
            request.default_style.font_size = 32.0;
            request.default_style.line_height = line_height;
            let mut state = backend.create_state();
            let layout = backend.layout_paragraph(&mut state, request);
            layout.lines[0].clone()
        };

        // A CJK fallback asks for more than one em of ascent alone, so a line
        // box of exactly `font_size` would clip the bottom of every glyph.
        let natural = sized(LineHeight::Normal);
        assert!(natural.height > 32.0);
        assert!(natural.baseline > 0.0 && natural.baseline < natural.height);

        // Explicit line heights are honoured exactly, and the difference is
        // split evenly above and below the glyphs.
        let short = sized(LineHeight::Px(64.0));
        let tall = sized(LineHeight::Px(96.0));
        assert_eq!(short.height, 64.0);
        assert_eq!(tall.height, 96.0);
        assert!((tall.baseline - short.baseline - 16.0).abs() < 0.01);
    }
}
