use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, PoisonError, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use skia_safe::{
    FontMgr, FontStyle as SkFontStyle, FontStyle, Typeface, font_style,
    textlayout::{
        FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle as SkParagraphStyle,
        TextAlign as SkTextAlign, TextStyle as SkTextStyle, TypefaceFontProvider,
        paragraph::VisitorFlags,
    },
};
use xui_interface::{
    ComputedTextStyle, FontCollection as XuiFontCollection, FontData, FontFamily, FontQuery,
    FontStretch, FontWeight, GlyphFlags, GlyphInstance, GlyphRun, LineHeight, LineLayout,
    NoRasterizer, ParagraphLayout, Point, Rect, Shaper, TextAlign, TextBackend, TextCluster,
    TextLayoutConstraints, TextLayoutInput, TextOffset, TextOverflow, TextRange, WhiteSpace,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkiaFontId(pub u32);

/// The typefaces a [`SkiaTextBackend`] has handed out ids for, with their
/// font files, shared with renderers.
pub struct SkiaFonts {
    fonts: RwLock<HashMap<u32, SkiaFont>>,
    epoch: AtomicU64,
}

impl SkiaFonts {
    fn new() -> Self {
        Self {
            fonts: RwLock::new(HashMap::new()),
            epoch: AtomicU64::new(0),
        }
    }

    fn insert(&self, id: u32, font: SkiaFont) {
        let mut fonts = self.fonts.write().unwrap_or_else(PoisonError::into_inner);
        fonts.insert(id, font);
    }

    fn bump_epoch(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }
}

impl XuiFontCollection for SkiaFonts {
    type FontId = SkiaFontId;
    type Font = SkiaFont;

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }

    fn font(&self, id: SkiaFontId) -> Option<SkiaFont> {
        let fonts = self.fonts.read().unwrap_or_else(PoisonError::into_inner);
        fonts.get(&id.0).cloned()
    }
}

/// A typeface's font file, as Skia serialized it.
#[derive(Clone)]
pub struct SkiaFont {
    bytes: Arc<[u8]>,
    index: u32,
}

impl FontData for SkiaFont {
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn index(&self) -> u32 {
        self.index
    }
}

#[derive(Default)]
pub struct SkiaParagraphState {
    /// Keeping the paragraph alive makes SkParagraph's shaped representation
    /// available for inspection and future native-run rendering extensions.
    paragraph: Option<Paragraph>,
}

impl SkiaParagraphState {
    pub fn paragraph(&self) -> Option<&Paragraph> {
        self.paragraph.as_ref()
    }
}

/// XUI text services backed by SkParagraph/SkShaper. It has no glyph
/// rasterizer: the Skia renderer draws glyph outlines itself.
pub struct SkiaTextBackend {
    font_manager: FontMgr,
    custom_fonts: TypefaceFontProvider,
    fonts: Arc<SkiaFonts>,
    /// Typefaces already offered to `fonts`, by unique id.
    registered: HashSet<u32>,
}

impl SkiaTextBackend {
    /// `_scale_factor` is unused: layout is in logical pixels.
    pub fn new(_scale_factor: f32) -> Self {
        Self {
            font_manager: FontMgr::new(),
            custom_fonts: TypefaceFontProvider::new(),
            fonts: Arc::new(SkiaFonts::new()),
            registered: HashSet::new(),
        }
    }

    fn font_collection(&self) -> FontCollection {
        let mut collection = FontCollection::new();
        collection.set_default_font_manager(self.font_manager.clone(), None);
        collection.set_asset_font_manager(Some(self.custom_fonts.clone().into()));
        collection.enable_font_fallback();
        collection
    }

    /// Publishes `typeface`'s font file to [`SkiaFonts`]. A typeface Skia
    /// cannot serialize stays unresolvable, and renderers skip its runs.
    fn register_typeface(&mut self, typeface: Typeface) -> SkiaFontId {
        let id = typeface.unique_id();
        if self.registered.insert(id)
            && let Some((bytes, index)) = typeface.to_font_data()
        {
            self.fonts.insert(
                id,
                SkiaFont {
                    bytes: Arc::from(bytes),
                    index: index as u32,
                },
            );
        }
        SkiaFontId(id)
    }
}

impl Default for SkiaTextBackend {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl TextBackend for SkiaTextBackend {
    type FontId = SkiaFontId;
    type Fonts = SkiaFonts;
    type Shaper = Self;
    type Rasterizer = NoRasterizer<SkiaFontId>;

    fn fonts(&self) -> &Arc<SkiaFonts> {
        &self.fonts
    }

    fn shaper(&mut self) -> &mut Self {
        self
    }

    fn rasterizer(&self) -> Option<&Arc<NoRasterizer<SkiaFontId>>> {
        None
    }

    fn load_system_fonts(&mut self) {
        self.font_manager = FontMgr::new();
        self.fonts.bump_epoch();
    }

    fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> Option<SkiaFontId> {
        let typeface = self.font_manager.new_from_data(bytes.as_ref(), None)?;
        let id = typeface.unique_id();
        self.custom_fonts.register_typeface(typeface, None);
        self.registered.insert(id);
        self.fonts.insert(id, SkiaFont { bytes, index: 0 });
        self.fonts.bump_epoch();
        Some(SkiaFontId(id))
    }

    fn query(&mut self, query: &FontQuery) -> Option<SkiaFontId> {
        let style = sk_font_style(query.weight, query.stretch, query.style);
        for family in query_family_names(&query.families) {
            let typeface = match family {
                Some(name) => self
                    .custom_fonts
                    .match_family_style(name, style)
                    .or_else(|| self.font_manager.match_family_style(name, style)),
                None => self.font_manager.legacy_make_typeface(None, style),
            };
            if let Some(typeface) = typeface {
                return Some(self.register_typeface(typeface));
            }
        }
        None
    }
}

impl Shaper for SkiaTextBackend {
    type FontId = SkiaFontId;
    type State = SkiaParagraphState;

    fn create_state(&mut self) -> Self::State {
        SkiaParagraphState::default()
    }

    fn layout_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphLayout<SkiaFontId> {
        if input.text_box_style.max_lines == Some(0) {
            state.paragraph = None;
            return empty_layout();
        }

        let text_style = sk_text_style(&input.default_style, input.default_style.line_height);

        let mut paragraph_style = SkParagraphStyle::new();
        paragraph_style.set_text_style(&text_style);
        paragraph_style.set_text_align(match input.paragraph_style.align {
            TextAlign::Start => SkTextAlign::Start,
            TextAlign::Center => SkTextAlign::Center,
            TextAlign::End => SkTextAlign::End,
            TextAlign::Justify => SkTextAlign::Justify,
        });
        let max_lines = input
            .text_box_style
            .max_lines
            .or_else(|| (input.paragraph_style.white_space == WhiteSpace::NoWrap).then_some(1));
        paragraph_style.set_max_lines(max_lines);
        if input.text_box_style.overflow == TextOverflow::Ellipsis {
            paragraph_style.set_ellipsis("…");
        }

        let mut builder = ParagraphBuilder::new(&paragraph_style, self.font_collection());
        builder.push_style(&text_style);
        let text = input.text.as_str();
        for (range, run) in input.shape_runs.segments(text, 0..text.len()) {
            match run {
                None => {
                    builder.add_text(&text[range]);
                }
                Some(index) => {
                    let style = input.shape_runs.as_slice()[index]
                        .style
                        .apply(&input.default_style);
                    builder.push_style(&sk_text_style(&style, input.default_style.line_height));
                    builder.add_text(&text[range]);
                    builder.pop();
                }
            }
        }
        let mut paragraph = builder.build();
        let width = match input.constraints {
            TextLayoutConstraints::Definate(width) => width.max(0.0),
            TextLayoutConstraints::Unbound | TextLayoutConstraints::MinSize => 1_000_000.0,
        };
        paragraph.layout(width);
        match input.constraints {
            TextLayoutConstraints::Unbound => {
                paragraph.layout(paragraph.max_intrinsic_width().ceil().max(1.0));
            }
            TextLayoutConstraints::MinSize => {
                paragraph.layout(paragraph.min_intrinsic_width().ceil().max(1.0));
            }
            TextLayoutConstraints::Definate(_) => {}
        }

        let layout = self.extract_layout(&mut paragraph, &input);
        state.paragraph = Some(paragraph);
        layout
    }
}

impl SkiaTextBackend {
    fn extract_layout(
        &mut self,
        paragraph: &mut Paragraph,
        input: &TextLayoutInput,
    ) -> ParagraphLayout<SkiaFontId> {
        let metrics = paragraph.get_line_metrics();
        let did_ellipsize = paragraph.did_exceed_max_lines();
        let mut output = ParagraphLayout {
            lines: metrics
                .iter()
                .map(|line| LineLayout {
                    source_line: line.line_number,
                    text_range: byte_range(line.start_index, line.end_including_newline),
                    run_range: 0..0,
                    glyph_range: 0..0,
                    cluster_range: 0..0,
                    x: line.left as f32,
                    y: (line.baseline - line.ascent) as f32,
                    width: line.width as f32,
                    height: line.height as f32,
                    baseline: line.baseline as f32,
                    hard_break: line.hard_break,
                    ellipsized: false,
                })
                .collect(),
            runs: Vec::new(),
            glyphs: Vec::new(),
            clusters: Vec::new(),
        };
        if did_ellipsize && let Some(line) = output.lines.last_mut() {
            line.ellipsized = true;
        }

        let default_weight = input.default_style.font_weight;
        paragraph.extended_visit(|line_index, info| {
            let Some(info) = info else { return };
            let Some(line) = output.lines.get_mut(line_index) else {
                return;
            };
            let line_text_start = line.text_range.start.raw;

            let typeface = info.font().typeface();
            let font_id = self.register_typeface(typeface);
            let run_start = output.glyphs.len();
            let cluster_start = output.clusters.len();
            let positions = info.positions();
            let bounds = info.bounds();
            let utf8_starts = info.utf8_starts();
            let boundaries = sorted_boundaries(utf8_starts);
            let bidi_level = visual_bidi_level(utf8_starts);
            let origin = info.origin();
            let advance = info.advance();
            let is_whitespace = info.flags().contains(VisitorFlags::WHITE_SPACE);

            for (index, &glyph_id) in info.glyphs().iter().enumerate() {
                let position = positions.get(index).copied().unwrap_or_default();
                let draw_pos = Point::new(origin.x + position.x, origin.y + position.y);
                let next_x = positions
                    .get(index + 1)
                    .map(|position| position.x)
                    .unwrap_or(advance.width);
                let raw_start = utf8_starts.get(index).copied().unwrap_or(0) as usize;
                let raw_end = boundaries
                    .iter()
                    .copied()
                    .find(|boundary| *boundary > raw_start)
                    .unwrap_or(raw_start);
                let glyph_bounds = bounds.get(index).copied().unwrap_or_default();
                let advance_width = (next_x - position.x).abs();
                let hitbox = if advance_width > 0.0 {
                    Rect::new(
                        origin.x + position.x.min(next_x),
                        line.y,
                        advance_width,
                        line.height.max(1.0),
                    )
                } else {
                    Rect::new(
                        draw_pos.x + glyph_bounds.left,
                        line.y,
                        glyph_bounds.width().max(0.0),
                        line.height.max(1.0),
                    )
                };
                let text_range = byte_range(raw_start, raw_end);
                let cluster_index = output
                    .clusters
                    .last()
                    .filter(|cluster| {
                        cluster.source_line == line_index && cluster.text_range == text_range
                    })
                    .map(|_| output.clusters.len() - 1)
                    .unwrap_or_else(|| {
                        let cluster_index = output.clusters.len();
                        output.clusters.push(TextCluster {
                            source_line: line_index,
                            local_text_range: raw_start.saturating_sub(line_text_start)
                                ..raw_end.saturating_sub(line_text_start),
                            text_range,
                            glyph_range: output.glyphs.len()..output.glyphs.len(),
                            hitbox,
                        });
                        cluster_index
                    });
                let glyph_index = output.glyphs.len();
                output.glyphs.push(GlyphInstance {
                    glyph_id: glyph_id as u32,
                    draw_pos,
                    hitbox,
                    cluster: cluster_index,
                    flags: if is_whitespace {
                        GlyphFlags::WHITESPACE
                    } else if glyph_id == 0 {
                        GlyphFlags::MISSING
                    } else {
                        GlyphFlags::NONE
                    },
                });
                let cluster = &mut output.clusters[cluster_index];
                cluster.glyph_range.end = glyph_index + 1;
                cluster.hitbox = union_rect(cluster.hitbox, hitbox);
            }

            let run_end = output.glyphs.len();
            if run_end > run_start {
                let text_start = boundaries.first().copied().unwrap_or(0);
                let text_end = boundaries.last().copied().unwrap_or(text_start);
                output.runs.push(GlyphRun {
                    text_range: byte_range(text_start, text_end),
                    glyph_range: run_start..run_end,
                    font_id,
                    font_size: info.font().size(),
                    font_weight: default_weight,
                    style_id: 0,
                    bidi_level,
                });
                if line.run_range.is_empty() {
                    line.run_range = output.runs.len() - 1..output.runs.len();
                } else {
                    line.run_range.end = output.runs.len();
                }
                if line.glyph_range.is_empty() {
                    line.glyph_range = run_start..run_end;
                    line.cluster_range = cluster_start..output.clusters.len();
                } else {
                    line.glyph_range.end = run_end;
                    line.cluster_range.end = output.clusters.len();
                }
            }
        });
        output
    }
}

fn empty_layout<F>() -> ParagraphLayout<F> {
    ParagraphLayout {
        lines: Vec::new(),
        runs: Vec::new(),
        glyphs: Vec::new(),
        clusters: Vec::new(),
    }
}

/// A Skia text style for `style`, its lines `line_height` tall.
fn sk_text_style(style: &ComputedTextStyle, line_height: LineHeight) -> SkTextStyle {
    let mut text_style = SkTextStyle::new();
    text_style.set_font_size(style.font_size.max(1.0));
    text_style.set_font_style(sk_font_style(
        style.font_weight,
        FontStretch::Normal,
        style.font_style,
    ));
    let families = style_family_names(&style.font_family);
    if !families.is_empty() {
        text_style.set_font_families(&families);
    }
    text_style.set_letter_spacing(style.letter_spacing);
    text_style
        .set_height(computed_line_height(line_height, style.font_size) / style.font_size.max(1.0));
    text_style.set_height_override(true);
    text_style
}

fn computed_line_height(line_height: LineHeight, font_size: f32) -> f32 {
    match line_height {
        LineHeight::Normal => font_size,
        LineHeight::Px(value) => value,
        LineHeight::Em(value) => value * font_size,
    }
    .max(1.0)
}

fn byte_range(start: usize, end: usize) -> TextRange {
    TextRange::new(TextOffset::byte_offset(start), TextOffset::byte_offset(end))
}

fn sorted_boundaries(values: &[u32]) -> Vec<usize> {
    let mut values: Vec<_> = values.iter().map(|value| *value as usize).collect();
    values.sort_unstable();
    values.dedup();
    values
}

fn visual_bidi_level(utf8_starts: &[u32]) -> u8 {
    utf8_starts
        .windows(2)
        .find_map(|pair| (pair[0] != pair[1]).then_some(u8::from(pair[0] > pair[1])))
        .unwrap_or(0)
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let far_x = (left.x + left.width).max(right.x + right.width);
    let far_y = (left.y + left.height).max(right.y + right.height);
    Rect::new(x, y, far_x - x, far_y - y)
}

fn style_family_names(family: &FontFamily) -> Vec<String> {
    match family {
        FontFamily::System => Vec::new(),
        FontFamily::Named(name) => vec![name.clone()],
        FontFamily::Stack(names) => names.clone(),
    }
}

fn query_family_names(families: &[FontFamily]) -> Vec<Option<&str>> {
    let mut output = Vec::new();
    for family in families {
        match family {
            FontFamily::System => output.push(None),
            FontFamily::Named(name) => output.push(Some(name.as_str())),
            FontFamily::Stack(names) => output.extend(names.iter().map(|name| Some(name.as_str()))),
        }
    }
    if output.is_empty() {
        output.push(None);
    }
    output
}

pub(crate) fn sk_font_style(
    weight: FontWeight,
    stretch: FontStretch,
    style: xui_interface::FontStyle,
) -> SkFontStyle {
    let weight = font_style::Weight::from(match weight {
        FontWeight::Thin => 100,
        FontWeight::ExtraLight => 200,
        FontWeight::Light => 300,
        FontWeight::Normal => 400,
        FontWeight::Medium => 500,
        FontWeight::SemiBold => 600,
        FontWeight::Bold => 700,
        FontWeight::ExtraBold => 800,
        FontWeight::Black => 900,
        FontWeight::Number(value) => i32::from(value.clamp(1, 1000)),
    });
    let width = match stretch {
        FontStretch::UltraCondensed => font_style::Width::ULTRA_CONDENSED,
        FontStretch::ExtraCondensed => font_style::Width::EXTRA_CONDENSED,
        FontStretch::Condensed => font_style::Width::CONDENSED,
        FontStretch::SemiCondensed => font_style::Width::SEMI_CONDENSED,
        FontStretch::Normal => font_style::Width::NORMAL,
        FontStretch::SemiExpanded => font_style::Width::SEMI_EXPANDED,
        FontStretch::Expanded => font_style::Width::EXPANDED,
        FontStretch::ExtraExpanded => font_style::Width::EXTRA_EXPANDED,
        FontStretch::UltraExpanded => font_style::Width::ULTRA_EXPANDED,
    };
    let slant = match style {
        xui_interface::FontStyle::Normal => font_style::Slant::Upright,
        xui_interface::FontStyle::Italic => font_style::Slant::Italic,
        xui_interface::FontStyle::Oblique => font_style::Slant::Oblique,
    };
    FontStyle::new(weight, width, slant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{OverflowWrap, ParagraphStyle, TextBoxStyle, TextContent, TextStyle};

    #[test]
    fn exposes_positioned_runs_for_native_glyph_drawing() {
        let mut backend = SkiaTextBackend::new(2.0);
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                TextContent::from_static("Skia 文本"),
                TextLayoutConstraints::max_width(300.0),
                TextStyle::default().into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                0,
            ),
        );
        assert!(state.paragraph().is_some());
        assert!(!layout.lines.is_empty());
        assert!(!layout.runs.is_empty());
        for line in &layout.lines {
            for glyph in &layout.glyphs[line.glyph_range.clone()] {
                assert!(glyph.draw_pos.x.is_finite());
                assert!(glyph.draw_pos.y.is_finite());
                assert!(glyph.hitbox.y >= line.y);
                assert!(glyph.hitbox.y + glyph.hitbox.height <= line.y + line.height + 0.01);
            }
        }
        for run in &layout.runs {
            assert!(backend.fonts().font(run.font_id).is_some());
            assert!(run.font_size > 0.0);
            assert!(!run.glyph_range.is_empty());
        }
    }

    #[test]
    fn multiline_caret_uses_paragraph_local_line_coordinates() {
        let mut backend = SkiaTextBackend::new(1.0);
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                TextContent::from_static("first\nsecond"),
                TextLayoutConstraints::max_width(300.0),
                TextStyle::default().into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                0,
            ),
        );
        assert_eq!(layout.lines.len(), 2);
        let second = &layout.lines[1];
        let caret = layout
            .caret_rect(xui_interface::TextPosition {
                offset: TextOffset::byte_offset(second.text_range.start.raw),
                affinity: xui_interface::Affinity::Before,
            })
            .expect("second line caret");
        assert_eq!(caret.y, second.y);
        assert_eq!(caret.height, second.height.max(1.0));
    }

    #[test]
    fn preserves_rtl_run_direction_for_hit_testing() {
        let mut backend = SkiaTextBackend::new(1.0);
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                TextContent::from_static("אבג"),
                TextLayoutConstraints::max_width(300.0),
                TextStyle::default().into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                0,
            ),
        );
        assert!(layout.runs.iter().any(|run| run.bidi_level % 2 == 1));
    }

    #[test]
    fn honors_width_and_max_lines() {
        let mut backend = SkiaTextBackend::new(1.0);
        let mut state = backend.create_state();
        let layout = backend.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                TextContent::from_static("one two three four five six seven"),
                TextLayoutConstraints::max_width(45.0),
                TextStyle::default().into(),
                ParagraphStyle {
                    overflow_wrap: OverflowWrap::Anywhere,
                    ..ParagraphStyle::default()
                },
                TextBoxStyle {
                    overflow: TextOverflow::Ellipsis,
                    max_lines: Some(2),
                },
                0,
            ),
        );
        assert!(layout.lines.len() <= 2);
    }
}
