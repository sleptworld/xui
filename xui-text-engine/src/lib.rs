use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics,
    Shaping, Style as CosmicStyle, SwashCache, Weight, Wrap, fontdb,
};
use std::sync::Arc;
use xui_interface::{ComputedTextStyle, Point, Rect, text::*};

pub struct CosmicEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
    font_epoch: u64,
    xui_font_id: Vec<fontdb::ID>,
    scale: f32,
}

#[derive(Default)]
pub struct CosmicParagraphState {
    buffer: Option<Buffer>,
}

impl CosmicEngine {
    pub fn new(init_scale_factor: f32) -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            font_epoch: 0,
            xui_font_id: Vec::new(),
            scale: init_scale_factor,
        }
    }

    pub fn font_system(&self) -> &FontSystem {
        &self.font_system
    }

    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        self.font_epoch = self.font_epoch.wrapping_add(1);
        &mut self.font_system
    }

    pub fn swash_cache(&self) -> &SwashCache {
        &self.swash_cache
    }

    pub fn swash_cache_mut(&mut self) -> &mut SwashCache {
        &mut self.swash_cache
    }

    fn create_buffer(
        &mut self,
        style: &ComputedTextStyle,
        constraints: TextLayoutConstraints,
    ) -> Buffer {
        let metrics = Metrics::new(
            style.font_size,
            line_height(style.line_height, style.font_size),
        );
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(width_for_constraints(constraints), None);
        buffer
    }

    fn layout_with_buffer(
        &mut self,
        buffer: &mut Buffer,
        input: TextLayoutInput,
    ) -> ParagraphLayout<CacheKey> {
        if input.text_box_style.max_lines == Some(0) {
            return ParagraphLayout {
                lines: Vec::new(),
                runs: Vec::new(),
                glyphs: Vec::new(),
                clusters: Vec::new(),
            };
        }
        let metrics = Metrics::new(
            input.default_style.font_size,
            line_height(
                input.default_style.line_height,
                input.default_style.font_size,
            ),
        );
        let width = width_for_constraints(input.constraints);
        let wrap = wrap_for_paragraph(&input.paragraph_style);
        let max_lines = input.text_box_style.max_lines;
        let height = match (input.text_box_style.overflow, max_lines) {
            (TextOverflow::Clip, Some(lines)) => Some(metrics.line_height * lines as f32),
            _ => None,
        };
        let ellipsize = match (input.text_box_style.overflow, max_lines, wrap, width) {
            (TextOverflow::Ellipsis, Some(lines), _, _) => {
                Ellipsize::End(EllipsizeHeightLimit::Lines(lines))
            }
            (TextOverflow::Ellipsis, None, Wrap::None, Some(_)) => {
                Ellipsize::End(EllipsizeHeightLimit::Lines(1))
            }
            _ => Ellipsize::None,
        };
        buffer.set_wrap(wrap);
        buffer.set_ellipsize(ellipsize);
        buffer.set_metrics_and_size(metrics, width, height);

        let attrs = attrs_for_style(&input.default_style);
        let text_len = input.text.as_str().len();
        buffer.set_text(
            input.text.as_str(),
            &attrs,
            Shaping::Advanced,
            align_for_paragraph(input.paragraph_style.align),
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        let line_bases = compute_line_base_byte_offsets(input.text.as_str());
        let mut layout = self.layout_paragraph_from_buffer(buffer, &line_bases);
        if input.text_box_style.overflow == TextOverflow::Clip {
            if let Some(max_lines) = input.text_box_style.max_lines {
                truncate_layout_lines(&mut layout, max_lines);
            }
        }
        if ellipsize != Ellipsize::None {
            if let Some(last) = layout.lines.last_mut() {
                last.ellipsized = last.text_range.end.raw < text_len;
            }
        }
        layout
    }

    fn layout_paragraph_from_buffer(
        &mut self,
        buffer: &cosmic_text::Buffer,
        line_base_byte_offsets: &[usize],
    ) -> ParagraphLayout<CacheKey> {
        let mut lines = Vec::new();
        let mut runs = Vec::new();
        let mut glyphs = Vec::new();
        let mut clusters = Vec::new();

        for cosmic_run in buffer.layout_runs() {
            let line_run_start = runs.len();
            let line_glyph_start = glyphs.len();
            let line_cluster_start = clusters.len();

            let line_base = line_base_byte_offsets
                .get(cosmic_run.line_i)
                .copied()
                .unwrap_or(0);

            let mut line_text_start: Option<usize> = None;
            let mut line_text_end: Option<usize> = None;

            let mut current_cluster_key: Option<ClusterKey> = None;
            let mut current_cluster_index: Option<usize> = None;

            let mut current_run_key: Option<RunKey> = None;
            let mut current_run_index: Option<usize> = None;

            for g in cosmic_run.glyphs {
                let global_start = line_base + g.start;
                let global_end = line_base + g.end;

                line_text_start = Some(match line_text_start {
                    Some(v) => v.min(global_start),
                    None => global_start,
                });

                line_text_end = Some(match line_text_end {
                    Some(v) => v.max(global_end),
                    None => global_end,
                });

                let cluster_key = ClusterKey {
                    source_line: cosmic_run.line_i,
                    start: g.start,
                    end: g.end,
                };

                let cluster_index = if current_cluster_key == Some(cluster_key) {
                    current_cluster_index.expect("current_cluster_index must exist")
                } else {
                    let cluster_index = clusters.len();
                    clusters.push(TextCluster {
                        source_line: cosmic_run.line_i,
                        local_text_range: g.start..g.end,
                        text_range: TextRange {
                            start: TextOffset::byte_offset(global_start),
                            end: TextOffset::byte_offset(global_end),
                        },
                        glyph_range: glyphs.len()..glyphs.len(),
                        hitbox: Rect::new(g.x, cosmic_run.line_top, g.w, cosmic_run.line_height),
                    });

                    current_cluster_key = Some(cluster_key);
                    current_cluster_index = Some(cluster_index);

                    cluster_index
                };

                let run_key = self.run_key_from_cosmic_glyph(&g);

                let run_index = if current_run_key.as_ref() == Some(&run_key) {
                    current_run_index.expect("current_run_index must exist")
                } else {
                    let run_index = runs.len();

                    runs.push(GlyphRun {
                        text_range: TextRange {
                            start: TextOffset::byte_offset(global_start),
                            end: TextOffset::byte_offset(global_end),
                        },
                        glyph_range: glyphs.len()..glyphs.len(),
                        font_id: run_key.font_id,
                        font_size: g.font_size,
                        font_weight: run_key.font_weight,
                        style_id: g.metadata as u32,
                        bidi_level: g.level.number(),
                    });

                    current_run_key = Some(run_key);
                    current_run_index = Some(run_index);

                    run_index
                };

                let glyph_index = glyphs.len();

                glyphs.push(GlyphInstance {
                    key: g.physical((0.0, cosmic_run.line_y), self.scale).cache_key,
                    glyph_id: g.glyph_id as u32,
                    draw_pos: Point::new(g.x_offset + g.x, cosmic_run.line_y + g.y_offset),
                    hitbox: Rect::new(g.x, cosmic_run.line_top, g.w, cosmic_run.line_height),
                    cluster: cluster_index,
                    flags: GlyphFlags::empty(),
                });

                let cluster = &mut clusters[cluster_index];
                if cluster.glyph_range.is_empty() {
                    cluster.glyph_range = glyph_index..glyph_index + 1;
                } else {
                    cluster.glyph_range.end = glyph_index + 1;
                }

                let left = cluster.hitbox.x.min(g.x);
                let right = (cluster.hitbox.x + cluster.hitbox.width).max(g.x + g.w);
                cluster.hitbox.x = left;
                cluster.hitbox.width = right - left;
                cluster.hitbox.y = cosmic_run.line_top;
                cluster.hitbox.height = cosmic_run.line_height;

                let glyph_run = &mut runs[run_index];
                if glyph_run.glyph_range.is_empty() {
                    glyph_run.glyph_range = glyph_index..glyph_index + 1;
                } else {
                    glyph_run.glyph_range.end = glyph_index + 1;
                }

                let run_start = glyph_run.text_range.start.raw;
                let run_end = glyph_run.text_range.end.raw;
                glyph_run.text_range = TextRange {
                    start: TextOffset::byte_offset(run_start.min(global_start)),
                    end: TextOffset::byte_offset(run_end.max(global_end)),
                };
            }

            let line_run_end = runs.len();
            let line_glyph_end = glyphs.len();
            let line_cluster_end = clusters.len();

            lines.push(LineLayout {
                source_line: cosmic_run.line_i,
                text_range: TextRange {
                    start: TextOffset::byte_offset(line_text_start.unwrap_or(line_base)),
                    end: TextOffset::byte_offset(line_text_end.unwrap_or(line_base)),
                },
                run_range: line_run_start..line_run_end,
                glyph_range: line_glyph_start..line_glyph_end,
                cluster_range: line_cluster_start..line_cluster_end,
                x: 0.0,
                y: cosmic_run.line_top,
                width: cosmic_run.line_w,
                height: cosmic_run.line_height,
                baseline: cosmic_run.line_y,
                hard_break: false,
                ellipsized: false,
            });
        }

        ParagraphLayout {
            lines,
            runs,
            glyphs,
            clusters,
        }
    }

    fn run_key_from_cosmic_glyph(&mut self, glyph: &cosmic_text::LayoutGlyph) -> RunKey {
        let bidi_level = glyph.level.number();

        RunKey {
            font_id: self.map_cosmic_font_id(glyph.font_id),
            font_size_bits: glyph.font_size.to_bits(),
            font_weight: map_cosmic_weight(glyph.font_weight),
            style_id: glyph.metadata as u32,
            bidi_level,
            direction: if bidi_level % 2 == 0 {
                TextDirection::Ltr
            } else {
                TextDirection::Rtl
            },
            script: None,
        }
    }

    fn map_cosmic_font_id(&mut self, id: fontdb::ID) -> xui_interface::FontId {
        if let Some(index) = self.xui_font_id.iter().position(|existing| *existing == id) {
            index as xui_interface::FontId
        } else {
            self.xui_font_id.push(id);
            (self.xui_font_id.len() - 1) as xui_interface::FontId
        }
    }

    fn font_handle(&self, id: fontdb::ID) -> Option<SystemFontHandle> {
        if self.font_system.db().face(id).is_none() {
            return None;
        }

        let raw = self
            .xui_font_id
            .iter()
            .position(|existing| *existing == id)
            .map(|index| index as u64)
            .or_else(|| id.to_string().parse::<u64>().ok())
            .unwrap_or(0);

        Some(SystemFontHandle(raw))
    }
}

fn align_for_paragraph(align: TextAlign) -> Option<Align> {
    match align {
        TextAlign::Start => None,
        TextAlign::Center => Some(Align::Center),
        TextAlign::End => Some(Align::End),
        TextAlign::Justify => Some(Align::Justified),
    }
}

fn wrap_for_paragraph(style: &ParagraphStyle) -> Wrap {
    match style.white_space {
        WhiteSpace::NoWrap | WhiteSpace::Pre => Wrap::None,
        WhiteSpace::Normal | WhiteSpace::PreWrap => match style.overflow_wrap {
            OverflowWrap::Normal => Wrap::Word,
            OverflowWrap::Anywhere => Wrap::Glyph,
            OverflowWrap::BreakWord => Wrap::WordOrGlyph,
        },
    }
}

fn truncate_layout_lines<K>(layout: &mut ParagraphLayout<K>, max_lines: usize) {
    if layout.lines.len() <= max_lines {
        return;
    }
    let (run_end, glyph_end, cluster_end) = layout
        .lines
        .get(max_lines.wrapping_sub(1))
        .map(|line| {
            (
                line.run_range.end,
                line.glyph_range.end,
                line.cluster_range.end,
            )
        })
        .unwrap_or((0, 0, 0));
    layout.lines.truncate(max_lines);
    layout.runs.truncate(run_end);
    layout.glyphs.truncate(glyph_end);
    layout.clusters.truncate(cluster_end);
}

impl Default for CosmicEngine {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Shaper for CosmicEngine {
    type State = CosmicParagraphState;
    type GlyphKey = CacheKey;

    fn create_state(&mut self) -> Self::State {
        CosmicParagraphState::default()
    }

    fn layout_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphLayout<Self::GlyphKey> {
        let mut buffer = state
            .buffer
            .take()
            .unwrap_or_else(|| self.create_buffer(&input.default_style, input.constraints));
        let layout = self.layout_with_buffer(&mut buffer, input);
        state.buffer = Some(buffer);
        layout
    }
}

impl FontDatabase for CosmicEngine {
    type FontId = fontdb::ID;

    fn epoch(&self) -> u64 {
        self.font_epoch
    }

    fn load_system_fonts(&mut self) {
        self.font_system = FontSystem::new();
        self.font_epoch = self.font_epoch.wrapping_add(1);
        self.xui_font_id.clear();
    }

    fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> Self::FontId {
        let ids = self
            .font_system
            .db_mut()
            .load_font_source(fontdb::Source::Binary(Arc::new(bytes.to_vec())));
        self.font_epoch = self.font_epoch.wrapping_add(1);

        ids.first().copied().unwrap_or_default()
    }

    fn query(&self, query: &FontQuery) -> Option<Self::FontId> {
        let families = query_families(&query.families);
        self.font_system.db().query(&fontdb::Query {
            families: &families,
            weight: font_weight(query.weight),
            stretch: font_stretch(query.stretch),
            style: font_style(query.style),
        })
    }

    fn font_data(&self, id: Self::FontId) -> Option<FontDataRef<'_>> {
        let face = self.font_system.db().face(id)?;
        match &face.source {
            fontdb::Source::Binary(data) => Some(FontDataRef::Bytes(data.as_ref().as_ref())),
            fontdb::Source::SharedFile(_, data) => Some(FontDataRef::Bytes(data.as_ref().as_ref())),
            fontdb::Source::File(_) => self.font_handle(id).map(FontDataRef::System),
        }
    }
}

impl GlyphRasterizer for CosmicEngine {
    type GlyphKey = CacheKey;

    fn rasterize(&mut self, key: Self::GlyphKey) -> Option<RasterizedGlyph> {
        let image = self
            .swash_cache
            .get_image(&mut self.font_system, key)
            .clone()?;

        Some(RasterizedGlyph {
            format: rasterized_glyph_format(image.content),
            width: image.placement.width as u32,
            height: image.placement.height as u32,
            left: image.placement.left,
            top: image.placement.top,
            pixels: Arc::from(rgba_bitmap_data(image.content, &image.data)),
        })
    }
}

impl TextBackend for CosmicEngine {}

fn attrs_for_style(style: &ComputedTextStyle) -> Attrs<'_> {
    let mut attrs = Attrs::new()
        .family(family(&style.font_family))
        .style(font_style(style.font_style))
        .weight(font_weight(style.font_weight));

    if style.decoration.underline {
        attrs = attrs.underline(cosmic_text::UnderlineStyle::Single);
    }

    attrs
}

fn family(family: &FontFamily) -> Family<'_> {
    match family {
        FontFamily::System => Family::SansSerif,
        FontFamily::Named(name) => Family::Name(name),
        FontFamily::Stack(names) => names
            .first()
            .map(|name| Family::Name(name.as_str()))
            .unwrap_or(Family::SansSerif),
    }
}

fn query_families(families: &[FontFamily]) -> Vec<Family<'_>> {
    if families.is_empty() {
        return vec![Family::SansSerif];
    }

    let mut output = Vec::new();
    for family in families {
        match family {
            FontFamily::System => output.push(Family::SansSerif),
            FontFamily::Named(name) => output.push(Family::Name(name)),
            FontFamily::Stack(names) => {
                output.extend(names.iter().map(|name| Family::Name(name.as_str())));
            }
        }
    }
    output
}

fn font_weight(weight: FontWeight) -> Weight {
    match weight {
        FontWeight::Thin => Weight::THIN,
        FontWeight::ExtraLight => Weight::EXTRA_LIGHT,
        FontWeight::Light => Weight::LIGHT,
        FontWeight::Normal => Weight::NORMAL,
        FontWeight::Medium => Weight::MEDIUM,
        FontWeight::SemiBold => Weight::SEMIBOLD,
        FontWeight::Bold => Weight::BOLD,
        FontWeight::ExtraBold => Weight::EXTRA_BOLD,
        FontWeight::Black => Weight::BLACK,
        FontWeight::Number(value) => Weight(value.clamp(1, 1000)),
    }
}

fn font_style(style: FontStyle) -> CosmicStyle {
    match style {
        FontStyle::Normal => CosmicStyle::Normal,
        FontStyle::Italic => CosmicStyle::Italic,
        FontStyle::Oblique => CosmicStyle::Oblique,
    }
}

fn font_stretch(stretch: FontStretch) -> fontdb::Stretch {
    match stretch {
        FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
        FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
        FontStretch::Condensed => fontdb::Stretch::Condensed,
        FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
        FontStretch::Normal => fontdb::Stretch::Normal,
        FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
        FontStretch::Expanded => fontdb::Stretch::Expanded,
        FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
        FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
    }
}

fn line_height(line_height: LineHeight, font_size: f32) -> f32 {
    match line_height {
        LineHeight::Normal => font_size,
        LineHeight::Px(px) => px,
        LineHeight::Em(em) => em * font_size,
    }
    .max(1.0)
}

fn map_cosmic_weight(weight: Weight) -> FontWeight {
    match weight {
        Weight::THIN => FontWeight::Thin,
        Weight::EXTRA_LIGHT => FontWeight::ExtraLight,
        Weight::LIGHT => FontWeight::Light,
        Weight::NORMAL => FontWeight::Normal,
        Weight::MEDIUM => FontWeight::Medium,
        Weight::SEMIBOLD => FontWeight::SemiBold,
        Weight::BOLD => FontWeight::Bold,
        Weight::EXTRA_BOLD => FontWeight::ExtraBold,
        Weight::BLACK => FontWeight::Black,
        Weight(value) => FontWeight::Number(value),
    }
}

fn width_for_constraints(constraints: TextLayoutConstraints) -> Option<f32> {
    match constraints {
        TextLayoutConstraints::Definate(width) if width.is_finite() => Some(width.max(0.0)),
        TextLayoutConstraints::Definate(_) | TextLayoutConstraints::Unbound => None,
        TextLayoutConstraints::MinSize => Some(0.0),
    }
}

fn rgba_bitmap_data(content: cosmic_text::SwashContent, data: &[u8]) -> Vec<u8> {
    match content {
        cosmic_text::SwashContent::Mask => {
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for alpha in data {
                rgba.extend_from_slice(&[*alpha, *alpha, *alpha, *alpha]);
            }
            rgba
        }
        cosmic_text::SwashContent::SubpixelMask => {
            let mut rgba = Vec::with_capacity(data.len() / 3 * 4);
            for rgb in data.chunks_exact(3) {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            rgba
        }
        cosmic_text::SwashContent::Color => data.to_vec(),
    }
}

fn rasterized_glyph_format(content: cosmic_text::SwashContent) -> RasterizedGlyphFormat {
    match content {
        cosmic_text::SwashContent::Mask => RasterizedGlyphFormat::Mask,
        cosmic_text::SwashContent::SubpixelMask => RasterizedGlyphFormat::SubpixelMask,
        cosmic_text::SwashContent::Color => RasterizedGlyphFormat::Color,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClusterKey {
    source_line: usize,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunKey {
    font_id: xui_interface::FontId,
    font_size_bits: u32,
    font_weight: FontWeight,
    style_id: TextStyleId,
    bidi_level: u8,
    direction: TextDirection,
    script: Option<Script>,
}

fn compute_line_base_byte_offsets(text: &str) -> Vec<usize> {
    let mut bases = Vec::new();
    bases.push(0);

    for (i, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            bases.push(i + 1);
        }
    }

    bases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(
        text: &'static str,
        width: f32,
        paragraph: ParagraphStyle,
        text_box: TextBoxStyle,
    ) -> ParagraphLayout<CacheKey> {
        let mut engine = CosmicEngine::default();
        let mut state = engine.create_state();
        let epoch = engine.epoch();
        engine.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                text.into(),
                TextLayoutConstraints::max_width(width),
                TextStyle::default().into(),
                paragraph,
                text_box,
                epoch,
            ),
        )
    }

    #[test]
    fn paragraph_options_map_to_cosmic_layout_modes() {
        assert_eq!(align_for_paragraph(TextAlign::Start), None);
        assert_eq!(
            align_for_paragraph(TextAlign::Center),
            Some(cosmic_text::Align::Center)
        );
        assert_eq!(
            align_for_paragraph(TextAlign::End),
            Some(cosmic_text::Align::End)
        );
        assert_eq!(
            align_for_paragraph(TextAlign::Justify),
            Some(cosmic_text::Align::Justified)
        );

        let mut paragraph = ParagraphStyle::default();
        paragraph.white_space = WhiteSpace::NoWrap;
        assert_eq!(wrap_for_paragraph(&paragraph), Wrap::None);
        paragraph.white_space = WhiteSpace::Normal;
        paragraph.overflow_wrap = OverflowWrap::Normal;
        assert_eq!(wrap_for_paragraph(&paragraph), Wrap::Word);
        paragraph.overflow_wrap = OverflowWrap::Anywhere;
        assert_eq!(wrap_for_paragraph(&paragraph), Wrap::Glyph);
        paragraph.overflow_wrap = OverflowWrap::BreakWord;
        assert_eq!(wrap_for_paragraph(&paragraph), Wrap::WordOrGlyph);
    }

    #[test]
    fn max_lines_zero_produces_an_empty_layout() {
        let mut text_box = TextBoxStyle::default();
        text_box.max_lines = Some(0);
        let layout = layout(
            "this text must not be shaped",
            80.0,
            ParagraphStyle::default(),
            text_box,
        );
        assert!(layout.lines.is_empty());
        assert!(layout.glyphs.is_empty());
    }

    #[test]
    fn wrapping_clip_and_ellipsis_limit_visible_lines() {
        let text = "one two three four five six seven eight nine ten";
        let wrapped = layout(
            text,
            55.0,
            ParagraphStyle::default(),
            TextBoxStyle::default(),
        );
        assert!(wrapped.lines.len() > 2);

        let mut clipped_box = TextBoxStyle::default();
        clipped_box.max_lines = Some(2);
        let clipped = layout(text, 55.0, ParagraphStyle::default(), clipped_box);
        assert_eq!(clipped.lines.len(), 2);
        assert!(!clipped.lines.last().unwrap().ellipsized);

        let mut ellipsis_box = TextBoxStyle::default();
        ellipsis_box.max_lines = Some(2);
        ellipsis_box.overflow = TextOverflow::Ellipsis;
        let ellipsized = layout(text, 55.0, ParagraphStyle::default(), ellipsis_box);
        assert_eq!(ellipsized.lines.len(), 2);
        assert!(ellipsized.lines.last().unwrap().ellipsized);
    }

    #[test]
    fn no_wrap_ellipsis_stays_on_one_line() {
        let mut paragraph = ParagraphStyle::default();
        paragraph.white_space = WhiteSpace::NoWrap;
        let mut text_box = TextBoxStyle::default();
        text_box.overflow = TextOverflow::Ellipsis;
        let value = layout("a deliberately long single line", 45.0, paragraph, text_box);
        assert_eq!(value.lines.len(), 1);
        assert!(value.lines[0].ellipsized);
        assert!(value.lines[0].width <= 45.0);
    }
}
