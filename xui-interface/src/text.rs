use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use crate::{Color, ComputedTextStyle, NodeLifecycleEvent, Point, Rect, Size};

pub trait Shaper {
    type State;
    type GlyphKey: Clone + Eq + Hash;

    fn create_state(&mut self) -> Self::State;
    fn layout_paragraph(
        &mut self,
        state: &mut Self::State,
        input: TextLayoutInput,
    ) -> ParagraphLayout<Self::GlyphKey>;

    fn handle_node_lifecycle(&mut self, _event: &NodeLifecycleEvent) {}
}

pub trait FontDatabase {
    type FontId: Copy + Eq + Hash;
    fn epoch(&self) -> u64;
    fn load_system_fonts(&mut self);
    fn load_font_bytes(&mut self, bytes: Arc<[u8]>) -> Self::FontId;
    fn query(&self, query: &FontQuery) -> Option<Self::FontId>;
    fn font_data(&self, id: Self::FontId) -> Option<FontDataRef<'_>>;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontDataRef<'a> {
    Bytes(&'a [u8]),
    System(SystemFontHandle),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SystemFontHandle(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuantizedSize(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubpixelOffset {
    Zero,
    Quarter,
    Half,
    ThreeQuarter,
}

#[derive(Debug, Clone)]
pub struct RasterizedGlyph {
    pub format: RasterizedGlyphFormat,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub pixels: Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterizedGlyphFormat {
    Mask,
    SubpixelMask,
    Color,
}

pub trait GlyphRasterizer {
    type GlyphKey;
    fn rasterize(&mut self, key: Self::GlyphKey) -> Option<RasterizedGlyph>;
}

pub trait TextBackend:
    FontDatabase + Shaper + GlyphRasterizer<GlyphKey = <Self as Shaper>::GlyphKey>
{
}

#[derive(Debug, Clone, PartialEq)]
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

impl Default for TextProps {
    fn default() -> Self {
        Self {
            text: TextContent::default(),
            style: TextStyle::default(),
            paragraph: ParagraphStyle::default(),
            text_box: TextBoxStyle::default(),
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
pub enum TextContent {
    Static(&'static str),
    Shared(Arc<str>),
}

impl TextContent {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(text) => text,
            Self::Shared(text) => text,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextDecoration {
    pub underline: bool,
    pub line_through: bool,
}

impl Default for TextDecoration {
    fn default() -> Self {
        Self {
            underline: false,
            line_through: false,
        }
    }
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

#[derive(Debug, Clone)]
pub struct ParagraphLayout<K = ()> {
    pub lines: Vec<LineLayout>,
    pub runs: Vec<GlyphRun>,
    pub glyphs: Vec<GlyphInstance<K>>,
    pub clusters: Vec<TextCluster>,
}

impl<K> ParagraphLayout<K> {
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
}

#[derive(Debug, Clone)]
pub struct LineLayout {
    pub source_line: usize,
    pub text_range: TextRange,
    pub run_range: std::ops::Range<usize>,

    pub glyph_range: std::ops::Range<usize>,
    pub cluster_range: std::ops::Range<usize>,

    pub x: f32,
    pub y: f32,

    pub width: f32,
    pub height: f32,

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
pub struct GlyphRun {
    pub text_range: TextRange,
    pub glyph_range: Range<usize>,

    pub font_id: FontId,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub style_id: TextStyleId,
    pub bidi_level: u8,
}

#[derive(Debug, Clone)]
pub struct GlyphInstance<K = ()> {
    pub key: K,
    pub glyph_id: GlyphId,
    pub draw_pos: Point,
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
    pub font_context_revision: u64,
    // pub scale_factor: f32,
}

impl TextLayoutInput {
    pub fn new(
        text: TextContent,
        constraints: TextLayoutConstraints,
        default_style: ComputedTextStyle,
        paragraph_style: ParagraphStyle,
        font_context_revision: u64,
        // scale_factor: f32,
    ) -> Self {
        Self {
            text,
            constraints,
            default_style,
            paragraph_style,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextLayoutKey {
    pub text_revision: u64,
    pub style_revision: u64,
    pub layout_style_hash: u64,

    pub max_width_bits: u32,
    pub max_height_bits: u32,

    pub scale_factor_bits: u32,
    pub font_context_revision: u64,
}
