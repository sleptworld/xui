use std::hash::{Hash, Hasher};
use std::sync::Arc;
use swash::shape::ShapeContext;
use swash::{
    FontRef, GlyphId, NormalizedCoord, Style, Weight,
    scale::{Render, ScaleContext, Source, StrikeWith, image::Content as SwashContent},
    zeno::{Format, Vector},
};
use xui_interface::{
    ComputedTextStyle, FontFamily, FontStyle as XuiFontStyle, FontWeight as XuiFontWeight,
    GlyphBitmap, GlyphPlacement, LineHeight, NodeId, Point, PositionedGlyph, Size,
    TextLayoutBackend, TextLayoutConstraints, TextMeasurer,
};

use crate::{
    bidi::BidiResolver,
    doc::{Direction, Doc, SpanStyle},
    fontique_library::{FamilyList, FontContext},
    line_breaker::Alignment,
    par::{BuilderState, Par, Session},
    span::{Span, SpanElement},
};

pub trait TextLayouter: TextMeasurer {
    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par>;

    fn layout_node_text(
        &mut self,
        _node_id: xui_interface::NodeId,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par> {
        self.layout_text(text, style, constraints)
    }

    fn get_cached_layout(&self, _node_id: NodeId) -> Option<Arc<Par>> {
        None
    }
}

const RASTER_SOURCES: &[Source] = &[
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::ColorOutline(0),
    Source::Outline,
];

#[derive(Clone)]
pub struct NativeGlyphKey {
    font: crate::fontique_library::Font,
    font_id: swash::CacheKey,
    glyph_id: GlyphId,
    subpx: [SubpixelOffset; 2],
    font_size_bits: u32,
    coords: Vec<NormalizedCoord>,
}

impl PartialEq for NativeGlyphKey {
    fn eq(&self, other: &Self) -> bool {
        self.font_id == other.font_id
            && self.glyph_id == other.glyph_id
            && self.subpx == other.subpx
            && self.font_size_bits == other.font_size_bits
            && self.coords == other.coords
    }
}

impl Eq for NativeGlyphKey {}

impl Hash for NativeGlyphKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.font_id.hash(state);
        self.glyph_id.hash(state);
        self.subpx.hash(state);
        self.font_size_bits.hash(state);
        self.coords.hash(state);
    }
}

#[derive(Hash, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
enum SubpixelOffset {
    Zero = 0,
    Quarter = 1,
    Half = 2,
    ThreeQuarters = 3,
}

impl SubpixelOffset {
    fn quantize(pos: f32) -> (i32, Self) {
        let base = pos.floor();
        let subpx = ((pos - base) * 8.0) as i32;
        let offset = match subpx {
            1..=2 => Self::Quarter,
            3..=4 => Self::Half,
            5..=6 => Self::ThreeQuarters,
            _ => Self::Zero,
        };
        (base as i32, offset)
    }

    fn to_f32(self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::ThreeQuarters => 0.75,
        }
    }
}

pub struct Engine {
    pub(crate) font_ctx: FontContext,
    pub(crate) bidi: BidiResolver,
    pub(crate) scx: ShapeContext,
    pub(crate) state: BuilderState,
    scale_factor: f32,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            scx: ShapeContext::new(),
            bidi: BidiResolver::new(),
            font_ctx: FontContext::default(),
            state: BuilderState::default(),
            scale_factor: 1.0,
        }
    }

    pub fn start<'a>(
        &'a mut self,
        dir: Direction,
        lang: Option<swash::text::Language>,
        offset: usize,
    ) -> Session<'a> {
        self.state.clear();
        self.state.begin(dir, lang, offset);
        let default_family = FamilyList::new("system-ui, sans-serif");
        let default_font = self.font_ctx.register_group(
            default_family.names(),
            default_family.key(),
            Default::default(),
        );
        if let Some(root) = self.state.spans.first_mut() {
            root.font_family = default_family;
            root.font = default_font;
        }
        Session {
            engine: self,
            dir_depth: 0,
            needs_bidi: false,
            last_offset: offset,
            dir: dir,
        }
    }

    #[inline(always)]
    pub fn measure_text_style(&mut self, text: &str, style: &ComputedTextStyle) -> Size<f32> {
        self.measure_text_style_with_constraints(text, style, TextLayoutConstraints::UNBOUNDED)
    }

    #[inline(always)]
    pub fn measure_text_style_with_constraints(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        size_for_par(&TextLayouter::layout_text(self, text, style, constraints))
    }

    fn layout_text_uncached(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Par {
        let styles = span_styles_for_style(style);
        let doc = Doc::simple(styles.iter(), text);
        self.layout_doc_with_constraints(&doc, constraints)
    }

    pub fn layout_doc(&mut self, doc: &Doc<'_>) -> Par {
        self.layout_doc_with_constraints(doc, TextLayoutConstraints::UNBOUNDED)
    }

    pub fn layout_doc_with_constraints(
        &mut self,
        doc: &Doc<'_>,
        constraints: TextLayoutConstraints,
    ) -> Par {
        let mut session = self.start(Direction::Auto, None, 0);
        session.process(doc);
        let mut par = session.finish(None);
        par.break_lines()
            .break_remaining(max_advance(constraints), Alignment::Start);
        par
    }

    pub fn measure_doc(&mut self, doc: &Doc<'_>) -> Size<f32> {
        let par = self.layout_doc(doc);
        size_for_par(&par)
    }
}

impl TextLayouter for Engine {
    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Arc<Par> {
        Arc::new(self.layout_text_uncached(text, style, constraints))
    }
}

impl TextLayoutBackend for Engine {
    type Layout = Arc<Par>;
    type GlyphKey = NativeGlyphKey;

    fn layout_text(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Self::Layout {
        TextLayouter::layout_text(self, text, style, constraints)
    }

    fn layout_size(&self, layout: &Self::Layout) -> Size<f32> {
        size_for_par(layout)
    }

    fn visit_layout_glyphs(
        &self,
        layout: &Self::Layout,
        origin: Point,
        scale_factor: f32,
        visitor: &mut dyn FnMut(PositionedGlyph<Self::GlyphKey>),
    ) {
        for line in layout.lines() {
            let baseline_y = origin.y + line.baseline();
            let mut pen_x = origin.x + line.offset();

            for run in line.runs() {
                let font = run.font().clone();
                let font_id = font.cache_key();
                let font_size = run.font_size() * scale_factor;
                let font_size_bits = font_size.to_bits();
                let coords = run.normalized_coords().to_vec();

                for cluster in run.visual_clusters() {
                    for glyph in cluster.glyphs() {
                        let x = (pen_x + glyph.x) * scale_factor;
                        let y = (baseline_y - glyph.y) * scale_factor;
                        pen_x += glyph.advance;

                        let (physical_x, subpx_x) = SubpixelOffset::quantize(x);
                        let (physical_y, subpx_y) = SubpixelOffset::quantize(y);
                        visitor(PositionedGlyph {
                            key: NativeGlyphKey {
                                font: font.clone(),
                                font_id,
                                glyph_id: glyph.id,
                                subpx: [subpx_x, subpx_y],
                                font_size_bits,
                                coords: coords.clone(),
                            },
                            physical_x,
                            physical_y,
                        });
                    }
                }
            }
        }
    }

    fn rasterize_glyph(&mut self, key: &Self::GlyphKey) -> Option<GlyphBitmap> {
        rasterize_swash_glyph(
            key.font.as_ref(),
            key.glyph_id,
            f32::from_bits(key.font_size_bits),
            &key.coords,
            Vector::new(key.subpx[0].to_f32(), key.subpx[1].to_f32()),
        )
    }
}

impl TextMeasurer for Engine {
    fn measure_text(&mut self, text: &str, style: &ComputedTextStyle) -> Size<f32> {
        self.measure_text_style(text, style)
    }

    fn measure_text_with_constraints(
        &mut self,
        text: &str,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Size<f32> {
        self.measure_text_style_with_constraints(text, style, constraints)
    }
}

fn max_advance(constraints: TextLayoutConstraints) -> f32 {
    constraints
        .max_width
        .filter(|width| width.is_finite())
        .map(|width| width.max(0.0))
        .unwrap_or(f32::MAX)
}

fn size_for_par(par: &Par) -> Size<f32> {
    let mut width: f32 = 0.0;
    let mut height = 0.0;
    for line in par.lines() {
        width = width.max(line.advance_without_trailing_whitespace());
        height += line.size();
    }

    Size::<f32>::new(width, height)
}

fn rasterize_swash_glyph(
    font: FontRef<'_>,
    glyph_id: GlyphId,
    font_size: f32,
    coords: &[NormalizedCoord],
    offset: Vector,
) -> Option<GlyphBitmap> {
    let mut context = ScaleContext::new();
    let mut image = swash::scale::image::Image::new();
    let mut scaler = context
        .builder(font)
        .hint(cfg!(not(target_os = "macos")))
        .size(font_size)
        .normalized_coords(coords)
        .build();

    let embolden = if cfg!(target_os = "macos") { 0.25 } else { 0.0 };

    if !Render::new(RASTER_SOURCES)
        .format(Format::CustomSubpixel([0.3, 0.0, -0.3]))
        .offset(offset)
        .embolden(embolden)
        .render_into(&mut scaler, glyph_id, &mut image)
    {
        return None;
    }

    let placement = image.placement;

    let (data, ptype) = rgba_bitmap_data(image.content, &image.data);

    Some(GlyphBitmap {
        ptype,
        data,
        width: placement.width as u32,
        height: placement.height as u32,
        placement: GlyphPlacement {
            left: placement.left,
            top: placement.top,
            width: placement.width as u32,
            height: placement.height as u32,
        },
    })
}

fn rgba_bitmap_data(content: SwashContent, data: &[u8]) -> (Vec<u8>, u32) {
    match content {
        SwashContent::Mask => {
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for alpha in data {
                rgba.extend_from_slice(&[*alpha, *alpha, *alpha, *alpha]);
            }
            (rgba, 0)
        }
        SwashContent::SubpixelMask => (data.to_vec(), 1),
        SwashContent::Color => (data.to_vec(), 2),
    }
}

fn span_styles_for_style(style: &ComputedTextStyle) -> Vec<SpanStyle<'static>> {
    let mut styles = Vec::with_capacity(7);
    styles.push(SpanStyle::FamilyList(family_list(&style.font_family)));
    styles.push(SpanStyle::Size(style.font_size));
    styles.push(SpanStyle::Weight(font_weight(style.font_weight)));
    styles.push(SpanStyle::Style(font_style(style.font_style)));
    styles.push(SpanStyle::LineSpacing(line_spacing(
        style.line_height,
        style.font_size,
    )));
    styles.push(SpanStyle::LetterSpacing(style.letter_spacing));
    styles.push(SpanStyle::Underline(style.decoration.underline));
    styles
}

fn family_list(family: &FontFamily) -> FamilyList {
    match family {
        FontFamily::System => FamilyList::new("system-ui, sans-serif"),
        FontFamily::Named(name) => FamilyList::new(name),
        FontFamily::Stack(names) => FamilyList::new(&names.join(", ")),
    }
}

fn font_weight(weight: XuiFontWeight) -> Weight {
    match weight {
        XuiFontWeight::Thin => Weight::THIN,
        XuiFontWeight::ExtraLight => Weight::EXTRA_LIGHT,
        XuiFontWeight::Light => Weight::LIGHT,
        XuiFontWeight::Normal => Weight::NORMAL,
        XuiFontWeight::Medium => Weight::MEDIUM,
        XuiFontWeight::SemiBold => Weight::SEMI_BOLD,
        XuiFontWeight::Bold => Weight::BOLD,
        XuiFontWeight::ExtraBold => Weight::EXTRA_BOLD,
        XuiFontWeight::Black => Weight::BLACK,
        XuiFontWeight::Number(value) => Weight(value.clamp(1, 1000)),
    }
}

fn font_style(style: XuiFontStyle) -> Style {
    match style {
        XuiFontStyle::Normal => Style::Normal,
        XuiFontStyle::Italic => Style::Italic,
        XuiFontStyle::Oblique => Style::from_degrees(14.0),
    }
}

fn line_spacing(line_height: LineHeight, font_size: f32) -> f32 {
    match line_height {
        LineHeight::Normal => 1.0,
        LineHeight::Px(px) => {
            if font_size > 0.0 {
                px / font_size
            } else {
                1.0
            }
        }
        LineHeight::Em(em) => em,
    }
}

impl<'a> Session<'a> {
    pub fn process(&mut self, doc: &Doc) {
        for root in &doc.roots {
            let span = &doc.spans[*root];
            self.layout_span(span, doc);
        }
    }

    fn layout_span(&mut self, span: &Span, doc: &Doc) {
        self.push_span(&span.properties);
        for e in &span.elements {
            match e {
                SpanElement::Span(i) => self.layout_span(&doc.spans[*i], doc),
                SpanElement::Fragment(i) => {
                    let (start, end) = doc.fragments[*i];
                    if start < end {
                        if let Some(s) = doc.text.get(start..end) {
                            self.add_text(s);
                        }
                    }
                }
            }
        }
        self.pop_span();
    }
}

#[cfg(test)]
mod test {
    use xui_interface::{ComputedTextStyle, TextLayoutConstraints, TextStyle};

    use crate::engine::{Engine, TextLayouter};

    fn text_style() -> ComputedTextStyle {
        TextStyle::default().into()
    }

    #[test]
    fn measured_width_keeps_increment_on_one_line_when_reused_as_constraint() {
        let mut engine = Engine::new();
        let style = text_style();

        let measured = engine.measure_text_style("Increment", &style);
        let par = engine.layout_text(
            "Increment",
            &style,
            TextLayoutConstraints::max_width(measured.width),
        );
        eprintln!("measured={measured:?}");
        for (line_index, line) in par.lines().enumerate() {
            eprintln!("line {line_index} advance={}", line.advance());
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    for glyph in cluster.glyphs() {
                        eprintln!("glyph id={} advance={}", glyph.id, glyph.advance);
                    }
                }
            }
        }

        assert_eq!(par.lines().count(), 1);
    }

    #[test]
    fn constrained_layout_keeps_all_source_text_across_lines() {
        let mut engine = Engine::new();
        let style = text_style();
        let text = "你好吗, FUCK THE WORLD";
        let par = engine.layout_text(text, &style, TextLayoutConstraints::max_width(80.0));

        let mut covered = vec![false; text.len()];
        for line in par.lines() {
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    for index in cluster.range() {
                        if let Some(slot) = covered.get_mut(index) {
                            *slot = true;
                        }
                    }
                }
            }
        }

        for (index, ch) in text.char_indices() {
            assert!(
                covered[index],
                "missing source text from layout at byte {index}: {ch:?}"
            );
        }
    }
}

//     #[test]
//     fn test() {
//         let mut engine = Engine::new();
//         let mut session = engine.start(crate::doc::Direction::Ltr, 2.0, 0);
//         let properties = &[SpanStyle::FamilyList(FamilyList::new("pingfang sc"))];
//         let doc = Doc::simple(properties, "Hello, World");
//         session.process(&doc);
//         let _par = session.finish(None);
//     }

//     #[test]
//     fn layout_text_reuses_cached_par_for_same_props() {
//         let mut engine = Engine::new();
//         let style = text_style();

//         let first = engine.layout_text("Hello, cache", &style, TextLayoutConstraints::UNBOUNDED,1.);
//         let second = engine.layout_text("Hello, cache", &style, TextLayoutConstraints::UNBOUNDED,1.);

//         assert!(Arc::ptr_eq(&first, &second));
//         assert_eq!(engine.layout_cache.len(), 1);
//     }

//     #[test]
//     fn layout_text_uses_style_in_cache_key() {
//         let mut engine = Engine::new();
//         let style = text_style();
//         let mut larger = style.clone();
//         larger.font_size += 1.0;
//         let mut spaced = style.clone();
//         spaced.letter_spacing = 1.0;

//         let first = engine.layout_text("Hello, cache", &style, TextLayoutConstraints::UNBOUNDED, 1.0);
//         let larger = engine.layout_text("Hello, cache", &larger, TextLayoutConstraints::UNBOUNDED,1.);
//         let spaced = engine.layout_text("Hello, cache", &spaced, TextLayoutConstraints::UNBOUNDED,1.);

//         assert!(!Arc::ptr_eq(&first, &larger));
//         assert!(!Arc::ptr_eq(&first, &spaced));
//         assert_eq!(engine.layout_cache.len(), 3);
//     }

//     #[test]
//     fn layout_text_uses_constraints_in_cache_key() {
//         let mut engine = Engine::new();
//         let style = text_style();

//         let wide = engine.layout_text(
//             "Hello, cache constraints",
//             &style,
//             TextLayoutConstraints::max_width(500.0),
//             1.
//         );
//         let narrow = engine.layout_text(
//             "Hello, cache constraints",
//             &style,
//             TextLayoutConstraints::max_width(50.0),
//             1.
//         );
//         let wide_again = engine.layout_text(
//             "Hello, cache constraints",
//             &style,
//             TextLayoutConstraints::max_width(500.0),
//             1.
//         );

//         assert!(Arc::ptr_eq(&wide, &wide_again));
//         assert!(!Arc::ptr_eq(&wide, &narrow));
//         assert_eq!(engine.layout_cache.len(), 2);
//     }

//     #[test]
//     fn measure_text_populates_layout_cache() {
//         let mut engine = Engine::new();
//         let style = text_style();

//         let _ = engine.measure_text("Measured once", &style,1.);
//         let first = engine.layout_text("Measured once", &style, TextLayoutConstraints::UNBOUNDED,1.);
//         let second = engine.layout_text("Measured once", &style, TextLayoutConstraints::UNBOUNDED,1.);

//         assert!(Arc::ptr_eq(&first, &second));
//         assert_eq!(engine.layout_cache.len(), 1);
//     }
// }
