//! Computed style values: what a [`units`](super::units) declaration resolves
//! to once the theme has been applied. Renderers read these; nothing here is
//! a declaration, so nothing here is a `StyleValue`.

use std::sync::Arc;

use super::units::{BlendMode, ColorMatrix, FilterQuality, StrokeLineStyle};
use crate::{Affine, Bounds, Color, ImageData, ImageKey, Point, Size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: ComputedColorStyle,
    pub line_style: StrokeLineStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
}

impl Shadow {
    pub fn visual_expansion(&self) -> f32 {
        self.offset.x.abs().max(self.offset.y.abs()) + self.blur * 3.0 + self.spread.max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComputedMaskShape {
    Rect,
    RoundedRect(f32),
    Circle,
    Ellipse,
    Line { from: Point, to: Point },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ComputedBackdropMask {
    #[default]
    None,
    Shape {
        shape: ComputedMaskShape,
        transform: Affine,
    },
    AlphaTexture {
        texture: ImageKey,
        transform: Affine,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComputedBackdropFilter {
    Blur {
        sigma_x: f32,
        sigma_y: f32,
        quality: FilterQuality,
    },
    Saturate(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    Sepia(f32),
    HueRotate(f32),
    Invert(f32),
    ColorMatrix(ColorMatrix),
    Pixelate {
        size: Size<f32>,
    },
    Refraction {
        strength: f32,
        chromatic_aberration: f32,
    },
    ChromaticAberration {
        offset: [f32; 2],
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedBackdropStyle {
    pub filters: Arc<[ComputedBackdropFilter]>,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub mask: ComputedBackdropMask,
}

impl Default for ComputedBackdropStyle {
    fn default() -> Self {
        Self {
            filters: Arc::from([]),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: ComputedBackdropMask::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputedEffect {
    Blur {
        sigma_x: f32,
        sigma_y: f32,
        quality: FilterQuality,
    },
    DropShadow {
        color: Color,
        offset: Point,
        sigma_x: f32,
        sigma_y: f32,
        spread: f32,
        quality: FilterQuality,
    },
    ColorMatrix(ColorMatrix),
    ImageMask {
        image: ImageKey,
        data: ImageData,
        bounds: Bounds,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComputedColorStyle {
    Solid(Color),
    LinearGradient(ComputedLinearGradientStyle),
    RadialGradient(ComputedRadialGradientStyle),
}

impl Default for ComputedColorStyle {
    fn default() -> Self {
        Self::Solid(Color::TRANSPARENT)
    }
}

impl From<Color> for ComputedColorStyle {
    fn from(value: Color) -> Self {
        Self::Solid(value)
    }
}

impl ComputedColorStyle {
    pub fn is_visible(self) -> bool {
        match self {
            Self::Solid(color) => color.a > 0.0,
            Self::LinearGradient(gradient) => gradient.from.a > 0.0 || gradient.to.a > 0.0,
            Self::RadialGradient(gradient) => gradient.from.a > 0.0 || gradient.to.a > 0.0,
        }
    }

    pub fn solid_color(self) -> Option<Color> {
        match self {
            Self::Solid(color) => Some(color),
            Self::LinearGradient(_) | Self::RadialGradient(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedLinearGradientStyle {
    pub start: Point,
    pub end: Point,
    pub from: Color,
    pub to: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedRadialGradientStyle {
    pub center: Point,
    pub radius: f32,
    pub from: Color,
    pub to: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedStrokeStyle {
    pub color: ComputedColorStyle,
    pub width: f32,
    pub line_style: StrokeLineStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedShadowStyle {
    pub color: Color,
    pub offset: Point,
    pub blur: f32,
    pub spread: f32,
}
