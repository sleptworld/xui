//! Production-oriented font selection, shaping and glyph rendering backend
//! for XUI.
//!
//! Responsibilities are separated into platform font management (`font`),
//! Unicode bidi resolution (`bidi`), HarfRust shaping (`shape`) and paragraph
//! line layout (`layout`). Glyph rendering is [`xui_swash`]'s
//! [`SwashRasterizer`] over this backend's [`FFonts`].
//!
//! The backend is the shaper (`TextBackend::Shaper = FBackend`). Its fonts and
//! rasterizer are shared, `Send + Sync` handles: a renderer keeps clones of
//! the `Arc`s and reads fonts or rasterizes glyphs on its own thread while the
//! backend keeps laying text out.

mod bidi;
mod font;
mod layout;
mod shape;
mod types;

use std::sync::Arc;

use xui_interface::{
    FontQuery, ParagraphLayout, ParagraphMetrics, Shaper, TextBackend, TextLayoutInput,
};
use xui_swash::SwashRasterizer;

pub use font::{FFont, FFonts};
pub use layout::{FHardLayout, FParagraphState};
pub use types::FFontId;

/// Fontique font management, HarfRust shaping and paragraph layout, with a
/// swash rasterizer over the same fonts.
pub struct FBackend {
    fonts: font::FontStore,
    rasterizer: Arc<SwashRasterizer<FFonts>>,
}

impl FBackend {
    /// Creates a backend and indexes fonts reported by the platform source.
    pub fn new() -> Self {
        let mut backend = Self::with_store(font::FontStore::with_system_fonts(true));
        backend.fonts.warm_system_ui();
        backend
    }

    /// Creates an empty backend for applications that only use embedded fonts.
    pub fn without_system_fonts() -> Self {
        Self::with_store(font::FontStore::empty())
    }

    fn with_store(fonts: font::FontStore) -> Self {
        Self {
            rasterizer: Arc::new(SwashRasterizer::new(fonts.shared().clone())),
            fonts,
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

impl Shaper for FBackend {
    type FontId = FFontId;
    type State = FParagraphState;

    fn create_state(&mut self) -> Self::State {
        FParagraphState::default()
    }

    fn layout_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphLayout<FFontId> {
        layout::layout(&mut self.fonts, state, input)
    }

    fn measure_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphMetrics {
        layout::measure(&mut self.fonts, state, input)
    }
}

impl TextBackend for FBackend {
    type FontId = FFontId;
    type Fonts = FFonts;
    type Shaper = Self;
    type Rasterizer = SwashRasterizer<FFonts>;

    fn fonts(&self) -> &Arc<FFonts> {
        self.fonts.shared()
    }

    fn shaper(&mut self) -> &mut Self {
        self
    }

    fn rasterizer(&self) -> Option<&Arc<SwashRasterizer<FFonts>>> {
        Some(&self.rasterizer)
    }

    fn load_system_fonts(&mut self) {
        self.fonts.load_system_fonts();
        self.fonts.warm_system_ui();
    }

    fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> Option<FFontId> {
        self.fonts.load_font_bytes(bytes)
    }

    fn query(&mut self, query: &FontQuery) -> Option<FFontId> {
        self.fonts.query(query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{
        ComputedTextStyle, FontFamily, FontStretch, FontStyle, FontWeight, LineHeight,
        OverflowWrap, ParagraphStyle, ShapeRun, ShapeRuns, ShapeStyle, TextAlign, TextBoxStyle,
        TextContent, TextLayoutConstraints, TextOverflow, TextStyle, WhiteSpace,
    };
    use xui_interface::{
        FontCollection, FontData, GlyphRasterizer, GlyphRequest, RasterOptions,
        RasterizedGlyphFormat, SubpixelOffset,
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
    fn measuring_matches_the_layout() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        let texts = [
            "",
            "row 7 / 10",
            "The quick brown fox jumps over the lazy dog, twice over.",
            "first line\nsecond, longer line\r\nthird",
            "English العربية עברית 中文 😀",
            "officeffi   trailing   ",
        ];
        let constraints = [
            TextLayoutConstraints::UNBOUNDED,
            TextLayoutConstraints::MIN_SIZE,
            TextLayoutConstraints::max_width(0.0),
            TextLayoutConstraints::max_width(60.0),
            TextLayoutConstraints::max_width(400.0),
        ];
        for text in texts {
            for constraint in constraints {
                for align in [TextAlign::Start, TextAlign::Center, TextAlign::Justify] {
                    for (max_lines, overflow, white_space) in [
                        (None, TextOverflow::Clip, WhiteSpace::Normal),
                        (Some(1), TextOverflow::Ellipsis, WhiteSpace::Normal),
                        (None, TextOverflow::Ellipsis, WhiteSpace::NoWrap),
                        (Some(0), TextOverflow::Clip, WhiteSpace::Normal),
                    ] {
                        let mut input = input(text, 0.0);
                        input.constraints = constraint;
                        input.paragraph_style.align = align;
                        input.paragraph_style.white_space = white_space;
                        input.text_box_style.max_lines = max_lines;
                        input.text_box_style.overflow = overflow;
                        // A fresh state and a reused one, as hosts probe.
                        let mut state = backend.create_state();
                        let expected = backend.layout_paragraph(&mut state, input.clone());
                        let measured = backend.measure_paragraph(&mut state, input.clone());
                        let mut fresh = backend.create_state();
                        let first = backend.measure_paragraph(&mut fresh, input.clone());
                        let case = format!(
                            "{text:?} {constraint:?} {align:?} {max_lines:?} {white_space:?}"
                        );
                        assert_eq!(measured, expected.metrics(), "{case}");
                        assert_eq!(first, expected.metrics(), "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn text_laid_out_at_its_measured_width_stays_on_one_line() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        for text in [
            "row 7 / 10",
            "The quick brown fox jumps over the lazy dog.",
            "中文排版测试，标点符号。",
            "English العربية עברית 中文",
        ] {
            let mut state = backend.create_state();
            let mut unbounded = input(text, 0.0);
            unbounded.constraints = TextLayoutConstraints::UNBOUNDED;
            let width = backend.measure_paragraph(&mut state, unbounded).size.width;
            let layout = backend.layout_paragraph(&mut state, input(text, width));
            assert_eq!(layout.lines.len(), 1, "{text:?} at {width}");
        }
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

    /// `(font, glyph, font size)` of every glyph with ink.
    fn inked(layout: &ParagraphLayout<FFontId>) -> Vec<(FFontId, u32, f32)> {
        layout
            .runs
            .iter()
            .flat_map(|run| {
                layout.glyphs[run.glyph_range.clone()]
                    .iter()
                    .filter(|glyph| !glyph.flags.contains(xui_interface::GlyphFlags::WHITESPACE))
                    .map(|glyph| (run.font_id, glyph.glyph_id, run.font_size))
            })
            .collect()
    }

    fn request(font: FFontId, glyph: u32, ppem: f32) -> GlyphRequest<FFontId> {
        GlyphRequest::new(
            font,
            glyph,
            ppem,
            SubpixelOffset::Zero,
            RasterOptions::default(),
        )
    }

    #[test]
    fn rasterizes_shaped_glyphs_at_the_requested_size() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(&mut state, input("Ag 中", 400.0));
        let glyphs = inked(&layout);
        assert_eq!(glyphs.len(), 3);
        let rasterizer = backend.rasterizer().unwrap().clone();

        let (font, glyph, size) = glyphs[0];
        let at_1x = rasterizer
            .rasterize(&request(font, glyph, size))
            .expect("'A' has ink");
        assert_eq!(at_1x.format, RasterizedGlyphFormat::Mask);
        assert_eq!(
            at_1x.pixels.len(),
            at_1x.width as usize * at_1x.height as usize
        );
        assert!(at_1x.pixels.iter().any(|value| *value > 0));
        assert!(at_1x.top > 0, "'A' sits above the baseline");

        let at_2x = rasterizer
            .rasterize(&request(font, glyph, size * 2.0))
            .unwrap();
        assert!(at_2x.height >= at_1x.height * 2 - 2);

        let space = layout
            .glyphs
            .iter()
            .find(|glyph| glyph.flags.contains(xui_interface::GlyphFlags::WHITESPACE))
            .unwrap();
        assert!(
            rasterizer
                .rasterize(&request(font, space.glyph_id, size))
                .is_none()
        );
        // The font data renderers read is the face the glyphs came from.
        let data = backend.fonts().font(font).unwrap();
        assert!(!data.bytes().is_empty());
    }

    #[test]
    fn rasterizes_on_other_threads_while_shaping_continues() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<SwashRasterizer<FFonts>>();
        send_sync::<FBackend>();

        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        // Taken before any CJK fallback face is resolved: faces registered
        // later must still reach the handle.
        let rasterizer = backend.rasterizer().unwrap().clone();
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(&mut state, input("Ag 中文 العربية", 400.0));
        let glyphs = inked(&layout);

        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let rasterizer = rasterizer.clone();
                let glyphs = glyphs.clone();
                std::thread::spawn(move || {
                    let scale = 1.0 + worker as f32 * 0.5;
                    glyphs.iter().all(|&(font, glyph, size)| {
                        rasterizer
                            .rasterize(&request(font, glyph, size * scale))
                            .is_some()
                    })
                })
            })
            .collect();
        // Shaping keeps going, resolving new fallback faces, meanwhile.
        for text in ["日本語 テキスト", "עברית", "Ελληνικά 😀", "हिन्दी"]
        {
            let mut state = backend.create_state();
            assert!(
                !backend
                    .layout_paragraph(&mut state, input(text, 200.0))
                    .lines
                    .is_empty()
            );
        }
        for worker in workers {
            assert!(
                worker.join().unwrap(),
                "every inked glyph renders on every thread"
            );
        }
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

    fn shape_run(range: std::ops::Range<usize>, edit: impl FnOnce(&mut ShapeStyle)) -> ShapeRun {
        let mut style = ShapeStyle::of(&ComputedTextStyle::from(TextStyle::default()));
        edit(&mut style);
        ShapeRun { range, style }
    }

    #[test]
    fn shape_runs_split_glyph_runs_and_report_their_style() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        // "plain " is 0..6, "bold" 6..10.
        let runs = ShapeRuns::new(vec![shape_run(6..10, |style| {
            style.font_weight = FontWeight::Bold;
        })]);
        let request = input("plain bold", 400.0).with_shape_runs(runs);
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(&mut state, request);

        for run in &layout.runs {
            let start = run.text_range.start.offset();
            let expected = if start >= 6 {
                (1, FontWeight::Bold)
            } else {
                (0, FontWeight::Normal)
            };
            assert_eq!((run.style_id, run.font_weight), expected, "run at {start}");
            assert!(
                run.text_range.end.offset() <= 6 || start >= 6,
                "a glyph run crosses the shape run boundary"
            );
        }
        assert!(layout.runs.iter().any(|run| run.style_id == 1));
    }

    #[test]
    fn a_larger_shape_run_makes_its_line_taller() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        let mut line = |runs: ShapeRuns| {
            let mut state = backend.create_state();
            let layout = backend
                .layout_paragraph(&mut state, input("small BIG", 400.0).with_shape_runs(runs));
            (layout.lines[0].height, layout.size().width)
        };
        let (plain_height, plain_width) = line(ShapeRuns::empty());
        let (big_height, big_width) = line(ShapeRuns::new(vec![shape_run(6..9, |style| {
            style.font_size *= 3.0;
        })]));
        assert!(
            big_height > plain_height * 2.0,
            "{big_height} vs {plain_height}"
        );
        assert!(big_width > plain_width, "{big_width} vs {plain_width}");
    }

    #[test]
    fn changing_shape_runs_reshapes_a_reused_state() {
        let mut backend = FBackend::new();
        if backend.face_count() == 0 {
            return;
        }
        let mut state = backend.create_state();
        let plain = backend.layout_paragraph(&mut state, input("reused state", 400.0));
        let bold = backend.layout_paragraph(
            &mut state,
            input("reused state", 400.0)
                .with_shape_runs(ShapeRuns::new(vec![shape_run(0..6, |style| {
                    style.font_size *= 2.0
                })])),
        );
        assert!(bold.size().width > plain.size().width);
    }
}
