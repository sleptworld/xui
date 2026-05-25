use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::Color;

#[derive(Debug, Clone, PartialEq)]
pub struct TextProps {
    pub text: TextContent,
    pub style: TextStyle,
    pub paragraph: ParagraphStyle,
    pub text_box: TextBoxStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextLayoutConstraints {
    pub max_width: Option<f32>,
}

impl TextLayoutConstraints {
    pub const UNBOUNDED: Self = Self { max_width: None };

    pub fn max_width(max_width: f32) -> Self {
        Self {
            max_width: Some(max_width),
        }
    }
}

impl Hash for TextLayoutConstraints {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.max_width.map(f32::to_bits).hash(state);
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
