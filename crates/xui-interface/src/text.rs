use std::convert::Infallible;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Range;
use std::sync::Arc;

use crate::{Color, ComputedTextStyle, NodeLifecycleEvent, Point, Rect, Size};

/// A text backend: fonts, shaping and (optionally) glyph rasterization, as
/// three parts with different ownership.
///
/// - [`fonts`](Self::fonts) is shared, read-only and thread-safe. Renderers
///   keep a clone of the `Arc` and resolve the fonts of any layout through it,
///   from any thread.
/// - [`shaper`](Self::shaper) is the mutable shaping context the host lays
///   text out with. A backend whose shaping state lives in the backend itself
///   uses `type Shaper = Self`.
/// - [`rasterizer`](Self::rasterizer), when present, renders glyphs from any
///   thread for renderers that draw bitmaps. Renderers that draw outlines
///   themselves (Skia, Vello) only need the fonts.
///
/// Every font id the backend hands out, in a layout or from
/// [`load_font_bytes`](Self::load_font_bytes) / [`query`](Self::query),
/// resolves through [`fonts`](Self::fonts) until its epoch changes.
pub trait TextBackend {
    type FontId: Copy + Eq + Hash + Debug + Send + Sync + 'static;
    type Fonts: FontCollection<FontId = Self::FontId>;
    type Shaper: Shaper<FontId = Self::FontId>;
    type Rasterizer: GlyphRasterizer<FontId = Self::FontId>;

    fn fonts(&self) -> &Arc<Self::Fonts>;
    fn shaper(&mut self) -> &mut Self::Shaper;
    /// `None` for backends that cannot render glyph bitmaps; their
    /// `Rasterizer` is usually [`NoRasterizer`].
    fn rasterizer(&self) -> Option<&Arc<Self::Rasterizer>>;

    /// Makes the platform's installed fonts available, if they are not yet.
    fn load_system_fonts(&mut self);
    /// Registers a font file. Returns the first face it contains, or `None`
    /// when the bytes are not a font.
    fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> Option<Self::FontId>;
    fn query(&mut self, query: &FontQuery) -> Option<Self::FontId>;
}

pub trait Shaper {
    type FontId: Copy + Eq + Hash;
    /// Persistent backend state for one logical text unit.
    ///
    /// Hosts create this once when the unit is created and reuse it across
    /// intrinsic and definite-width layout probes. Implementations must
    /// invalidate or rebuild cached internals when the input content, shaping
    /// style, or font context changes.
    type State;

    /// Creates the state for a newly created logical text unit.
    fn create_state(&mut self) -> Self::State;
    /// Lays a paragraph out in logical pixels. The result does not depend on
    /// the scale factor: physical sizes are a rasterization concern.
    fn layout_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphLayout<Self::FontId>;

    /// Measures a paragraph without laying its glyphs out: the result equals
    /// [`ParagraphLayout::metrics`] of what
    /// [`layout_paragraph`](Self::layout_paragraph) would return for the same
    /// state and input.
    ///
    /// Hosts measure under intrinsic constraints, and often several times per
    /// pass, so a backend that can size lines without placing glyphs should
    /// override this. The default lays the paragraph out.
    fn measure_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphMetrics {
        self.layout_paragraph(state, input).metrics()
    }

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

/// The fonts a backend has handed out ids for, shared with renderers.
pub trait FontCollection: Send + Sync {
    type FontId: Copy + Eq + Hash + Debug + Send + Sync + 'static;
    type Font: FontData;

    /// Advances whenever fonts are added or replaced. Layouts shaped under an
    /// older epoch may resolve fallbacks differently, and an id may refer to
    /// a different font, so caches keyed on font ids are dropped when it
    /// changes.
    fn epoch(&self) -> u64;
    fn font(&self, id: Self::FontId) -> Option<Self::Font>;
}

/// One font face's data, as the backend holds it.
///
/// Handles are cheap to clone (reference counted) and can be kept and sent
/// across threads, so a renderer can build its native font object once and
/// reuse it without copying the file.
pub trait FontData: Clone + Send + Sync + 'static {
    /// The whole font file or collection the face is in.
    fn bytes(&self) -> &[u8];
    /// Index of the face in a collection; renderers must preserve it.
    fn index(&self) -> u32;
    /// The platform's identity for an installed font, for renderers that
    /// prefer their native font manager. `None` for application fonts.
    fn system(&self) -> Option<SystemFontInfo<'_>> {
        None
    }
}

/// How the platform font manager knows an installed face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemFontInfo<'a> {
    pub family: &'a str,
    pub postscript_name: &'a str,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub stretch: FontStretch,
}

/// Uninhabited [`FontData`], for collections that never resolve a font.
impl FontData for Infallible {
    fn bytes(&self) -> &[u8] {
        match *self {}
    }

    fn index(&self) -> u32 {
        match *self {}
    }
}

/// A collection that resolves nothing, for backends without font data (test
/// doubles, measuring-only shapers).
pub struct NoFonts<F> {
    epoch: std::sync::atomic::AtomicU64,
    _font: PhantomData<fn() -> F>,
}

impl<F> NoFonts<F> {
    pub fn new() -> Self {
        Self {
            epoch: std::sync::atomic::AtomicU64::new(0),
            _font: PhantomData,
        }
    }

    /// Advances the epoch, as a backend does when its fonts change.
    pub fn bump_epoch(&self) {
        self.epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl<F> Default for NoFonts<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Copy + Eq + Hash + Debug + Send + Sync + 'static> FontCollection for NoFonts<F> {
    type FontId = F;
    type Font = Infallible;

    fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn font(&self, _id: F) -> Option<Infallible> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontQuery {
    pub families: Vec<FontFamily>,
    pub weight: FontWeight,
    pub style: FontStyle,
    pub stretch: FontStretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontStretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuantizedSize(pub u16);

/// Horizontal position of a glyph within its pixel, in quarters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubpixelOffset {
    Zero,
    Quarter,
    Half,
    ThreeQuarter,
}

impl SubpixelOffset {
    /// Splits a physical pen coordinate into the whole pixel a bitmap is
    /// placed at and the offset it is rendered with. Positions round to the
    /// nearest quarter, so `3.9` lands on pixel 4 with no offset.
    pub fn split(position: f32) -> (i32, Self) {
        let quarters = (position * 4.0).round();
        let whole = (quarters / 4.0).floor();
        let offset = match (quarters - whole * 4.0) as i32 {
            1 => Self::Quarter,
            2 => Self::Half,
            3 => Self::ThreeQuarter,
            _ => Self::Zero,
        };
        (whole as i32, offset)
    }

    pub const fn as_f32(self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Quarter => 0.25,
            Self::Half => 0.5,
            Self::ThreeQuarter => 0.75,
        }
    }
}

/// How a glyph outline is turned into pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RasterOptions {
    /// Apply TrueType/CFF hinting. Off on macOS by default, where text keeps
    /// its outline shapes, on elsewhere.
    pub hinting: bool,
    /// Produce RGB subpixel (LCD) coverage instead of a single alpha channel.
    pub subpixel_antialiasing: bool,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            hinting: !cfg!(target_os = "macos"),
            subpixel_antialiasing: false,
        }
    }
}

/// Everything that decides what a rendered glyph looks like.
///
/// Built by the renderer at draw time, never by the shaper: the physical size
/// and the subpixel offset depend on the scale factor, layer transforms and
/// the final pen position, none of which layout knows. Requests are plain
/// values, so renderers can key their own caches and atlases on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphRequest<F> {
    pub font: F,
    pub glyph: GlyphId,
    /// Physical pixels per em, as `f32` bits: the logical font size times
    /// every scale applied on the way to the screen.
    pub ppem_bits: u32,
    pub subpixel_x: SubpixelOffset,
    pub options: RasterOptions,
}

impl<F> GlyphRequest<F> {
    pub fn new(
        font: F,
        glyph: GlyphId,
        ppem: f32,
        subpixel_x: SubpixelOffset,
        options: RasterOptions,
    ) -> Self {
        Self {
            font,
            glyph,
            ppem_bits: ppem.to_bits(),
            subpixel_x,
            options,
        }
    }

    pub fn ppem(&self) -> f32 {
        f32::from_bits(self.ppem_bits)
    }
}

/// A rendered glyph in device pixels.
///
/// Pixels are row-major with no row padding, laid out by `format`:
///
/// - [`RasterizedGlyphFormat::Mask`]: one coverage byte per pixel.
/// - [`RasterizedGlyphFormat::SubpixelMask`]: RGBA, per-channel coverage in
///   R, G and B.
/// - [`RasterizedGlyphFormat::Color`]: RGBA with premultiplied alpha.
///
/// `left` is the offset of the bitmap's left edge from the pen position and
/// `top` the distance from the baseline up to its top edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphImage {
    pub format: RasterizedGlyphFormat,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub pixels: Arc<[u8]>,
}

impl GlyphImage {
    pub const fn bytes_per_pixel(&self) -> usize {
        self.format.bytes_per_pixel()
    }

    /// Bytes per row of `pixels`.
    pub const fn stride(&self) -> usize {
        self.width as usize * self.bytes_per_pixel()
    }
}

/// Renders glyphs from any thread.
///
/// Every input is in the request, so the result does not depend on backend
/// state such as a scale factor, and the rasterizer can be shared (`Arc`) with
/// render threads while shaping goes on elsewhere. Implementations cache their
/// results: a repeated request should cost a lookup and an `Arc` clone.
pub trait GlyphRasterizer: Send + Sync {
    type FontId: Copy + Eq + Hash + Send + Sync;

    /// `None` when the glyph has no ink (whitespace) or cannot be rendered.
    fn rasterize(&self, request: &GlyphRequest<Self::FontId>) -> Option<GlyphImage>;
}

/// The rasterizer of a backend that has none.
pub struct NoRasterizer<F>(PhantomData<fn() -> F>);

impl<F: Copy + Eq + Hash + Send + Sync> GlyphRasterizer for NoRasterizer<F> {
    type FontId = F;

    fn rasterize(&self, _request: &GlyphRequest<F>) -> Option<GlyphImage> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterizedGlyphFormat {
    Mask,
    SubpixelMask,
    Color,
}

impl RasterizedGlyphFormat {
    /// Bytes per pixel of a [`GlyphImage`] in this format.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Mask => 1,
            Self::SubpixelMask | Self::Color => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextProps {
    pub text: TextContent,
    pub style: TextStyle,
    pub paragraph: ParagraphStyle,
    pub text_box: TextBoxStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextLayoutConstraints {
    Definate(f32),
    #[default]
    Unbound,
    MinSize,
}

impl TextLayoutConstraints {
    pub const UNBOUNDED: Self = Self::Unbound;
    pub const MIN_SIZE: Self = Self::MinSize;
    pub const fn max_width(max_width: f32) -> Self {
        Self::Definate(max_width)
    }
}

impl TextProps {
    pub fn new(text: impl Into<TextContent>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }
}

impl Hash for TextProps {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.style.hash(state);
        self.paragraph.hash(state);
        self.text_box.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TextContent<T: Text = ()> {
    Static(&'static str),
    Shared(Arc<str>),
    Other(T),
}

pub trait Text: Debug + Clone + PartialEq + Eq + Hash {
    fn text(&self) -> &str;
}

impl Text for () {
    fn text(&self) -> &str {
        ""
    }
}

impl<T: Text> TextContent<T> {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(text) => text,
            Self::Shared(text) => text,
            Self::Other(text) => text.text(),
        }
    }

    pub fn from_static(text: &'static str) -> Self {
        Self::Static(text)
    }

    pub fn copy_from(text: &str) -> Self {
        Self::Shared(Arc::from(text))
    }
}

impl Default for TextContent {
    fn default() -> Self {
        Self::Static("")
    }
}

impl From<&'static str> for TextContent {
    fn from(value: &'static str) -> Self {
        Self::Static(value)
    }
}

impl<'a> From<&'a String> for TextContent {
    fn from(value: &'a String) -> Self {
        Self::Shared(Arc::from(value.as_str()))
    }
}

impl From<String> for TextContent {
    fn from(value: String) -> Self {
        Self::Shared(Arc::from(value))
    }
}

impl From<Arc<str>> for TextContent {
    fn from(value: Arc<str>) -> Self {
        Self::Shared(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub color: Color,
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing: f32,
    pub decoration: TextDecoration,
}

impl TextStyle {
    pub fn line_height(&self) -> f32 {
        match self.line_height {
            LineHeight::Normal => self.font_size,
            LineHeight::Px(px) => px,
            LineHeight::Em(em) => em * self.font_size,
        }
        .max(1.0)
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            font_family: FontFamily::System,
            font_size: 14.0,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            letter_spacing: 0.0,
            decoration: TextDecoration::default(),
        }
    }
}

impl From<&ComputedTextStyle> for TextStyle {
    fn from(style: &ComputedTextStyle) -> Self {
        Self {
            color: style.color,
            font_family: style.font_family.clone(),
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: style.line_height,
            letter_spacing: style.letter_spacing,
            decoration: style.decoration,
        }
    }
}

impl Hash for TextStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_color(self.color, state);
        self.font_family.hash(state);
        self.font_size.to_bits().hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.line_height.hash(state);
        self.letter_spacing.to_bits().hash(state);
        self.decoration.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FontFamily {
    System,
    Named(String),
    Stack(Vec<String>),
}

impl From<&'static str> for FontFamily {
    fn from(value: &'static str) -> Self {
        Self::Named(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Thin,
    ExtraLight,
    Light,
    Normal,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Black,
    Number(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    Normal,
    Px(f32),
    Em(f32),
}

impl Hash for LineHeight {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Normal => {}
            Self::Px(value) | Self::Em(value) => value.to_bits().hash(state),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextDecoration {
    pub underline: bool,
    pub line_through: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParagraphStyle {
    pub align: TextAlign,
    pub vertical_align: TextVerticalAlign,
    pub white_space: WhiteSpace,
    pub overflow_wrap: OverflowWrap,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            align: TextAlign::Start,
            vertical_align: TextVerticalAlign::Baseline,
            white_space: WhiteSpace::Normal,
            overflow_wrap: OverflowWrap::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Start,
    Center,
    End,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextVerticalAlign {
    Top,
    Middle,
    Bottom,
    Baseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhiteSpace {
    Normal,
    NoWrap,
    Pre,
    PreWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverflowWrap {
    Normal,
    Anywhere,
    BreakWord,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextBoxStyle {
    pub overflow: TextOverflow,
    pub max_lines: Option<usize>,
}

impl Default for TextBoxStyle {
    fn default() -> Self {
        Self {
            overflow: TextOverflow::Clip,
            max_lines: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
}

fn hash_color<H: Hasher>(color: Color, state: &mut H) {
    color.r.to_bits().hash(state);
    color.g.to_bits().hash(state);
    color.b.to_bits().hash(state);
    color.a.to_bits().hash(state);
}

/// A paragraph's measurements, without its glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParagraphMetrics {
    /// The widest line's width and the bottom of the last line.
    pub size: Size<f32>,
    pub first_baseline: Option<f32>,
    pub line_count: usize,
}

#[derive(Debug, Clone)]
pub struct ParagraphLayout<F> {
    pub lines: Vec<LineLayout>,
    pub runs: Vec<GlyphRun<F>>,
    pub glyphs: Vec<GlyphInstance>,
    pub clusters: Vec<TextCluster>,
}

impl<F> ParagraphLayout<F> {
    pub fn metrics(&self) -> ParagraphMetrics {
        ParagraphMetrics {
            size: self.size(),
            first_baseline: self.lines.first().map(|line| line.baseline),
            line_count: self.lines.len(),
        }
    }

    pub fn size(&self) -> Size<f32> {
        let width = self
            .lines
            .iter()
            .map(|line| line.width)
            .fold(0.0_f32, f32::max);
        let height = self
            .lines
            .iter()
            .map(|line| line.y + line.height)
            .fold(0.0_f32, f32::max);
        Size::new(width, height)
    }

    pub fn hit_test_point(&self, point: Point) -> Option<TextPosition> {
        let line = self.line_at_y(point.y)?;

        let clusters = self.clusters.get(line.cluster_range.clone())?;
        if clusters.is_empty() {
            return Some(self.x_to_position_from_glyphs(line, point.x));
        }

        for cluster in clusters {
            let left = cluster.hitbox.x;
            let right = cluster.hitbox.x + cluster.hitbox.width;
            if point.x >= left && point.x <= right {
                let mid = left + cluster.hitbox.width * 0.5;
                let rtl = self.cluster_is_rtl(cluster);
                return Some(if (point.x < mid) != rtl {
                    TextPosition {
                        offset: cluster.text_range.start,
                        affinity: Affinity::Before,
                    }
                } else {
                    TextPosition {
                        offset: cluster.text_range.end,
                        affinity: Affinity::After,
                    }
                });
            }
        }

        let nearest = clusters.iter().min_by(|left, right| {
            distance_to_rect_x(point.x, left.hitbox)
                .total_cmp(&distance_to_rect_x(point.x, right.hitbox))
        })?;
        let rtl = self.cluster_is_rtl(nearest);
        let before = point.x < nearest.hitbox.x;
        Some(if before != rtl {
            TextPosition {
                offset: nearest.text_range.start,
                affinity: Affinity::Before,
            }
        } else {
            TextPosition {
                offset: nearest.text_range.end,
                affinity: Affinity::After,
            }
        })
    }

    pub fn caret_rect(&self, position: TextPosition) -> Option<Rect> {
        let line = self.line_for_position(position)?;
        let height = line.height.max(1.0);
        let x = self.caret_x_for_offset(line, position.offset);
        Some(Rect::new(x, line.y, 1.0, height))
    }

    pub fn selection_rects(&self, range: TextRange) -> Vec<Rect> {
        if range.start.unit != range.end.unit || range.start.raw == range.end.raw {
            return Vec::new();
        }
        let (start, end) = if range.start.raw <= range.end.raw {
            (range.start, range.end)
        } else {
            (range.end, range.start)
        };

        self.lines
            .iter()
            .filter_map(|line| {
                let line_start = comparable_offset(line.text_range.start, start.unit)?;
                let line_end = comparable_offset(line.text_range.end, start.unit)?;
                if end.raw <= line_start || start.raw >= line_end {
                    return None;
                }
                let start_x = if start.raw <= line_start {
                    line.x
                } else {
                    self.caret_x_for_offset(line, start)
                };
                let end_x = if end.raw >= line_end {
                    line.x + line.width
                } else {
                    self.caret_x_for_offset(line, end)
                };
                let left = start_x.min(end_x);
                let right = start_x.max(end_x);
                (right > left).then(|| Rect::new(left, line.y, right - left, line.height.max(1.0)))
            })
            .collect()
    }

    fn line_at_y(&self, y: f32) -> Option<&LineLayout> {
        if let Some(line) = self
            .lines
            .iter()
            .find(|line| y >= line.y && y <= line.y + line.height)
        {
            return Some(line);
        }
        self.lines.iter().min_by(|left, right| {
            distance_to_range(y, left.y, left.y + left.height).total_cmp(&distance_to_range(
                y,
                right.y,
                right.y + right.height,
            ))
        })
    }

    fn line_for_position(&self, position: TextPosition) -> Option<&LineLayout> {
        let unit = position.offset.unit;
        let raw = position.offset.raw;
        let mut boundary_match = None;
        for line in &self.lines {
            let Some(start) = comparable_offset(line.text_range.start, unit) else {
                continue;
            };
            let Some(end) = comparable_offset(line.text_range.end, unit) else {
                continue;
            };
            if raw > start && raw < end {
                return Some(line);
            }
            if raw == start && position.affinity == Affinity::Before {
                return Some(line);
            }
            if raw == end && position.affinity == Affinity::After {
                return Some(line);
            }
            if raw == start || raw == end {
                boundary_match = Some(line);
            }
        }
        boundary_match.or_else(|| {
            self.lines.iter().min_by_key(|line| {
                let start = comparable_offset(line.text_range.start, unit).unwrap_or(0);
                let end = comparable_offset(line.text_range.end, unit).unwrap_or(start);
                raw.abs_diff(raw.clamp(start, end))
            })
        })
    }

    fn caret_x_for_offset(&self, line: &LineLayout, offset: TextOffset) -> f32 {
        if let Some(clusters) = self.clusters.get(line.cluster_range.clone()) {
            for cluster in clusters {
                let Some(start) = comparable_offset(cluster.text_range.start, offset.unit) else {
                    continue;
                };
                let Some(end) = comparable_offset(cluster.text_range.end, offset.unit) else {
                    continue;
                };
                if offset.raw >= start && offset.raw <= end {
                    let left = cluster.hitbox.x;
                    let right = left + cluster.hitbox.width;
                    let rtl = self.cluster_is_rtl(cluster);
                    return if offset.raw == start {
                        if rtl { right } else { left }
                    } else if rtl {
                        left
                    } else {
                        right
                    };
                }
            }
        }
        self.caret_x_from_glyphs(line, offset)
    }

    fn cluster_is_rtl(&self, cluster: &TextCluster) -> bool {
        self.runs
            .iter()
            .find(|run| ranges_overlap(&run.glyph_range, &cluster.glyph_range))
            .is_some_and(|run| run.bidi_level % 2 == 1)
    }

    fn caret_x_from_glyphs(&self, line: &LineLayout, offset: TextOffset) -> f32 {
        let glyph_range = line.glyph_range.clone();
        let visual_offset = if line.text_range.start.unit == offset.unit {
            offset.raw.saturating_sub(line.text_range.start.raw)
        } else {
            0
        };
        let mut x = line.x;
        let Some(glyphs) = self.glyphs.get(glyph_range) else {
            return x;
        };
        for (visual_index, glyph) in glyphs.iter().enumerate() {
            if visual_index >= visual_offset {
                return glyph.hitbox.x;
            }
            x = glyph.hitbox.x + glyph.hitbox.width;
        }
        x
    }

    fn x_to_position_from_glyphs(&self, line: &LineLayout, x: f32) -> TextPosition {
        let Some(glyphs) = self.glyphs.get(line.glyph_range.clone()) else {
            return TextPosition {
                offset: line.text_range.start,
                affinity: Affinity::Before,
            };
        };
        for (visual_index, glyph) in glyphs.iter().enumerate() {
            let mid = glyph.hitbox.x + glyph.hitbox.width * 0.5;
            if x < mid {
                return TextPosition {
                    offset: offset_add(line.text_range.start, visual_index),
                    affinity: Affinity::Before,
                };
            }
        }
        TextPosition {
            offset: line.text_range.end,
            affinity: Affinity::After,
        }
    }
}

fn distance_to_range(value: f32, start: f32, end: f32) -> f32 {
    if value < start {
        start - value
    } else if value > end {
        value - end
    } else {
        0.0
    }
}

fn distance_to_rect_x(x: f32, rect: Rect) -> f32 {
    distance_to_range(x, rect.x, rect.x + rect.width)
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn comparable_offset(offset: TextOffset, unit: TextOffsetUnit) -> Option<usize> {
    (offset.unit == unit).then_some(offset.raw)
}

fn offset_add(offset: TextOffset, delta: usize) -> TextOffset {
    TextOffset {
        raw: offset.raw.saturating_add(delta),
        unit: offset.unit,
    }
}

#[derive(Debug, Clone)]
pub struct LineLayout {
    pub source_line: usize,
    pub text_range: TextRange,
    pub run_range: std::ops::Range<usize>,

    pub glyph_range: std::ops::Range<usize>,
    pub cluster_range: std::ops::Range<usize>,

    /// Top-left of the line box in paragraph-local logical coordinates.
    pub x: f32,
    pub y: f32,

    pub width: f32,
    pub height: f32,

    /// Baseline Y in paragraph-local logical coordinates.
    pub baseline: f32,
    pub hard_break: bool,
    pub ellipsized: bool,
}

pub type FontId = u32;
pub type GlyphId = u32;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextDirection {
    Ltr,
    Rtl,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Script {
    Latin,
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Arabic,
    Hebrew,
    Devanagari,

    Common,
    Inherited,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GlyphRun<F> {
    pub text_range: TextRange,
    pub glyph_range: Range<usize>,

    pub font_id: F,
    /// Font size in logical pixels. Renderers apply the output scale through
    /// their canvas/device transform, not by modifying this value.
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub style_id: TextStyleId,
    pub bidi_level: u8,
}

#[derive(Debug, Clone)]
pub struct GlyphInstance {
    pub glyph_id: GlyphId,
    /// Glyph origin (the point on the baseline) in paragraph-local logical
    /// coordinates. A renderer can pass these positions directly to its native
    /// positioned-glyph API and add only the paragraph origin.
    pub draw_pos: Point,
    /// Logical layout/hit-test box, also paragraph-local. This is an advance
    /// box and does not need to match the glyph's ink bounds.
    pub hitbox: Rect,
    pub cluster: usize,
    pub flags: GlyphFlags,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct GlyphFlags: u16 {
        const NONE             = 0;
        const WHITESPACE       = 1 << 0;
        const LINE_BREAK       = 1 << 1;
        const TAB              = 1 << 2;
        const FALLBACK_FONT    = 1 << 3;
        const COLOR_GLYPH      = 1 << 4;
        const LIGATURE         = 1 << 5;
        const MARK             = 1 << 6;
        const INVISIBLE        = 1 << 7;
        const MISSING          = 1 << 8;
        const SYNTHETIC        = 1 << 9;
    }
}

#[derive(Debug, Clone)]
pub struct TextCluster {
    pub source_line: usize,

    pub local_text_range: Range<usize>,
    pub text_range: TextRange,
    pub glyph_range: Range<usize>,
    pub hitbox: Rect,
    // pub flags: ClusterFlags,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TextOffset {
    pub raw: usize,
    pub unit: TextOffsetUnit,
}

impl TextOffset {
    pub fn char_offset(offset: usize) -> Self {
        Self {
            raw: offset,
            unit: TextOffsetUnit::Char,
        }
    }

    pub fn byte_offset(offset: usize) -> Self {
        Self {
            raw: offset,
            unit: TextOffsetUnit::Utf8Byte,
        }
    }

    pub fn utf16_offset(offset: usize) -> Self {
        Self {
            raw: offset,
            unit: TextOffsetUnit::Utf16CodeUnit,
        }
    }

    pub fn offset(&self) -> usize {
        self.raw
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextOffsetUnit {
    Utf8Byte,
    Utf16CodeUnit,
    Char,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TextRange {
    pub start: TextOffset,
    pub end: TextOffset,
}

impl TextRange {
    pub fn new(start: TextOffset, end: TextOffset) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TextPosition {
    pub offset: TextOffset,
    pub affinity: Affinity,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Affinity {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLayoutInput {
    pub text: TextContent,
    pub constraints: TextLayoutConstraints,
    pub default_style: ComputedTextStyle,
    pub paragraph_style: ParagraphStyle,
    pub text_box_style: TextBoxStyle,
    pub font_context_revision: u64,
    // pub scale_factor: f32,
}

impl TextLayoutInput {
    pub fn new(
        text: TextContent,
        constraints: TextLayoutConstraints,
        default_style: ComputedTextStyle,
        paragraph_style: ParagraphStyle,
        text_box_style: TextBoxStyle,
        font_context_revision: u64,
        // scale_factor: f32,
    ) -> Self {
        Self {
            text,
            constraints,
            default_style,
            paragraph_style,
            text_box_style,
            font_context_revision,
            // scale_factor,
        }
    }
}

pub type TextStyleId = u32;

#[derive(Debug, Clone)]
pub struct TextSpan {
    pub range: TextRange,
    pub style_id: TextStyleId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextLayoutKey {
    pub text_revision: u64,
    pub style_revision: u64,
    pub layout_style_hash: u64,

    pub max_width_bits: u32,
    pub max_height_bits: u32,

    pub scale_factor_bits: u32,
    pub font_context_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subpixel_split_rounds_to_the_nearest_quarter() {
        use SubpixelOffset::*;
        assert_eq!(SubpixelOffset::split(3.0), (3, Zero));
        assert_eq!(SubpixelOffset::split(3.1), (3, Zero));
        assert_eq!(SubpixelOffset::split(3.2), (3, Quarter));
        assert_eq!(SubpixelOffset::split(3.5), (3, Half));
        assert_eq!(SubpixelOffset::split(3.7), (3, ThreeQuarter));
        assert_eq!(SubpixelOffset::split(3.9), (4, Zero));
        assert_eq!(SubpixelOffset::split(-0.3), (-1, ThreeQuarter));
        assert_eq!(ThreeQuarter.as_f32(), 0.75);
    }

    #[test]
    fn glyph_image_rows_follow_the_format() {
        let mask = GlyphImage {
            format: RasterizedGlyphFormat::Mask,
            width: 2,
            height: 1,
            left: 0,
            top: 1,
            pixels: Arc::from(&[10u8, 200][..]),
        };
        assert_eq!(mask.stride(), 2);
        let color = GlyphImage {
            format: RasterizedGlyphFormat::Color,
            pixels: Arc::from(&[1u8, 2, 3, 4, 5, 6, 7, 8][..]),
            ..mask
        };
        assert_eq!(color.stride(), 8);
    }

    fn multiline_layout() -> ParagraphLayout<u32> {
        let lines = vec![
            LineLayout {
                source_line: 0,
                text_range: TextRange::new(TextOffset::byte_offset(0), TextOffset::byte_offset(3)),
                run_range: 0..0,
                glyph_range: 0..0,
                cluster_range: 0..3,
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 20.0,
                baseline: 15.0,
                hard_break: true,
                ellipsized: false,
            },
            LineLayout {
                source_line: 1,
                text_range: TextRange::new(TextOffset::byte_offset(3), TextOffset::byte_offset(6)),
                run_range: 0..0,
                glyph_range: 0..0,
                cluster_range: 3..6,
                x: 0.0,
                y: 20.0,
                width: 30.0,
                height: 20.0,
                baseline: 35.0,
                hard_break: false,
                ellipsized: false,
            },
        ];
        let clusters = (0..6)
            .map(|offset| TextCluster {
                source_line: offset / 3,
                local_text_range: offset % 3..offset % 3 + 1,
                text_range: TextRange::new(
                    TextOffset::byte_offset(offset),
                    TextOffset::byte_offset(offset + 1),
                ),
                glyph_range: 0..0,
                hitbox: Rect::new(
                    (offset % 3) as f32 * 10.0,
                    (offset / 3) as f32 * 20.0,
                    10.0,
                    20.0,
                ),
            })
            .collect();
        ParagraphLayout {
            lines,
            runs: Vec::new(),
            glyphs: Vec::new(),
            clusters,
        }
    }

    #[test]
    fn caret_uses_the_line_containing_the_position() {
        let layout = multiline_layout();
        let caret = layout
            .caret_rect(TextPosition {
                offset: TextOffset::byte_offset(4),
                affinity: Affinity::Before,
            })
            .unwrap();
        assert_eq!(caret, Rect::new(10.0, 20.0, 1.0, 20.0));
    }

    #[test]
    fn selection_produces_one_rect_per_covered_line() {
        let layout = multiline_layout();
        assert_eq!(
            layout.selection_rects(TextRange::new(
                TextOffset::byte_offset(1),
                TextOffset::byte_offset(5),
            )),
            vec![
                Rect::new(10.0, 0.0, 20.0, 20.0),
                Rect::new(0.0, 20.0, 20.0, 20.0),
            ]
        );
    }

    #[test]
    fn hit_testing_below_the_paragraph_uses_the_last_line() {
        let layout = multiline_layout();
        assert_eq!(
            layout.hit_test_point(Point::new(1.0, 100.0)),
            Some(TextPosition {
                offset: TextOffset::byte_offset(3),
                affinity: Affinity::Before,
            })
        );
    }
}
