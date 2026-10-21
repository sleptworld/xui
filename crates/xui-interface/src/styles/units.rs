//! Style value units: what a style property is declared as, before the theme
//! resolves it.
//!
//! Units carry no notion of which node or capability uses them; a capability
//! (see [`super::caps`]) names its fields in these types.

use std::sync::Arc;

use crate::{Affine, Bounds, Color, ImageData, ImageKey, Point, Size};

/// A declared style value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StyleValue<T> {
    /// No declaration in this layer. Not the CSS keyword `unset`.
    #[default]
    Unset,
    Inherit,
    Initial,
    Value(T),
}

impl<T> StyleValue<T> {
    pub fn value(value: T) -> Self {
        Self::Value(value)
    }
}

/// What a style property accepts: a value of its unit — anything `Into` it —
/// or a declaration as such (`StyleValue::Inherit`, `Initial`, ..).
///
/// Implemented here, per unit type, rather than as two `Accepts` impls per
/// property in `xui-core`: there, neither `StyleValue` nor the unit is local,
/// so rustc cannot rule out `StyleValue<D>: Into<D>` and the two would
/// overlap. Here `StyleValue` is local and the question has an answer.
pub trait IntoStyleValue<D> {
    fn into_style_value(self) -> StyleValue<D>;
}

impl<D> IntoStyleValue<D> for StyleValue<D> {
    fn into_style_value(self) -> StyleValue<D> {
        self
    }
}

/// The unit types style properties are declared in. A new style capability
/// field in a new unit type adds it here.
macro_rules! into_style_value {
    ($($unit:ty),* $(,)?) => {
        $(
            impl<T: Into<$unit>> IntoStyleValue<$unit> for T {
                fn into_style_value(self) -> StyleValue<$unit> {
                    StyleValue::Value(self.into())
                }
            }
        )*
    };
}

into_style_value!(
    bool,
    f32,
    crate::core::Sizing,
    crate::core::EdgeInsets,
    LengthValue,
    ColorValue,
    ColorStyle,
    AlignStyle,
    JustifyStyle,
    PositionStyle,
    DisplayStyle,
    VisibilityStyle,
    StrokeStyle,
    ShadowStyle,
    BackdropStyle,
    Arc<[Effect]>,
    ScrollDirectionStyle,
    ScrollbarVisibilityStyle,
    crate::FontFamily,
    crate::FontWeight,
    crate::FontStyle,
    crate::LineHeight,
    crate::TextDecoration,
    crate::style::CursorIcon,
);

/// Layers `from` over `dest`: a declaration replaces, an absence keeps.
pub fn merge_value<T: Clone>(dest: &mut StyleValue<T>, from: &StyleValue<T>) {
    if !matches!(from, StyleValue::Unset) {
        *dest = from.clone();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorToken {
    Background,
    Foreground,
    Card,
    CardForeground,
    Popover,
    PopoverForeground,
    Primary,
    PrimaryForeground,
    Secondary,
    SecondaryForeground,
    /// Subdued surfaces such as a tab list or skeleton.
    Muted,
    /// Secondary text: descriptions, placeholders, inactive tabs.
    MutedForeground,
    /// Hover and highlight surface for menu items and ghost buttons.
    Accent,
    AccentForeground,
    Destructive,
    DestructiveForeground,
    Border,
    /// Border of form controls.
    Input,
    /// Focus ring.
    Ring,
    /// Alias of `Foreground`.
    Text,
    /// Alias of `PrimaryForeground`.
    InverseText,
    /// Alias of `Card`.
    Surface,
    /// Alias of `Muted`.
    MutedSurface,
}

impl ColorToken {
    /// This token with its alpha multiplied by `alpha`, like Tailwind's
    /// `bg-primary/90`.
    pub fn alpha(self, alpha: f32) -> ColorValue {
        ColorValue::TokenAlpha(self, alpha)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingToken {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadiusToken {
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontSizeToken {
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorValue {
    Color(Color),
    Token(ColorToken),
    /// A token whose resolved alpha is multiplied by the given factor.
    TokenAlpha(ColorToken, f32),
}

impl From<Color> for ColorValue {
    fn from(value: Color) -> Self {
        Self::Color(value)
    }
}

impl From<ColorToken> for ColorValue {
    fn from(value: ColorToken) -> Self {
        Self::Token(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorStyle {
    Solid(ColorValue),
    LinearGradient(LinearGradientStyle),
    RadialGradient(RadialGradientStyle),
}

impl Default for ColorStyle {
    fn default() -> Self {
        Self::Solid(Color::TRANSPARENT.into())
    }
}

impl From<ColorValue> for ColorStyle {
    fn from(value: ColorValue) -> Self {
        Self::Solid(value)
    }
}

impl From<Color> for ColorStyle {
    fn from(value: Color) -> Self {
        Self::Solid(value.into())
    }
}

impl From<ColorToken> for ColorStyle {
    fn from(value: ColorToken) -> Self {
        Self::Solid(value.into())
    }
}

impl ColorStyle {
    pub fn solid(color: impl Into<ColorValue>) -> Self {
        Self::Solid(color.into())
    }

    pub fn linear_gradient(
        start: Point,
        end: Point,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self::LinearGradient(LinearGradientStyle::new(start, end, from, to))
    }

    pub fn radial_gradient(
        center: Point,
        radius: impl Into<LengthValue>,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self::RadialGradient(RadialGradientStyle::new(center, radius, from, to))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrokeLineStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    pub color: ColorStyle,
    pub width: LengthValue,
    pub line_style: StrokeLineStyle,
}

impl StrokeStyle {
    pub fn new(color: impl Into<ColorStyle>, width: impl Into<LengthValue>) -> Self {
        Self {
            color: color.into(),
            width: width.into(),
            line_style: StrokeLineStyle::Solid,
        }
    }

    pub fn line_style(mut self, line_style: StrokeLineStyle) -> Self {
        self.line_style = line_style;
        self
    }

    pub fn dashed(mut self) -> Self {
        self.line_style = StrokeLineStyle::Dashed;
        self
    }

    pub fn dotted(mut self) -> Self {
        self.line_style = StrokeLineStyle::Dotted;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearGradientStyle {
    pub start: Point,
    pub end: Point,
    pub from: ColorValue,
    pub to: ColorValue,
}

impl LinearGradientStyle {
    pub fn new(
        start: Point,
        end: Point,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self {
            start,
            end,
            from: from.into(),
            to: to.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialGradientStyle {
    pub center: Point,
    pub radius: LengthValue,
    pub from: ColorValue,
    pub to: ColorValue,
}

impl RadialGradientStyle {
    pub fn new(
        center: Point,
        radius: impl Into<LengthValue>,
        from: impl Into<ColorValue>,
        to: impl Into<ColorValue>,
    ) -> Self {
        Self {
            center,
            radius: radius.into(),
            from: from.into(),
            to: to.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthValue {
    Px(f32),
    Spacing(SpacingToken),
    Radius(RadiusToken),
    FontSize(FontSizeToken),
}

/// A row-major 4x5 affine RGBA color matrix.
pub type ColorMatrix = [f32; 20];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FilterQuality {
    Low,
    #[default]
    Medium,
    High,
}

impl FilterQuality {
    pub const fn gaussian_support(self) -> f32 {
        match self {
            Self::Low => 2.0,
            Self::Medium => 3.0,
            Self::High => 4.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub const fn requires_destination_snapshot(self) -> bool {
        !matches!(self, Self::Normal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaskShape {
    Rect,
    RoundedRect(LengthValue),
    Circle,
    Ellipse,
    Line { from: Point, to: Point },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum BackdropMask {
    #[default]
    None,
    Shape {
        shape: MaskShape,
        transform: Affine,
    },
    AlphaTexture {
        texture: ImageKey,
        transform: Affine,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackdropFilter {
    Blur {
        sigma_x: LengthValue,
        sigma_y: LengthValue,
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
        size: Size<LengthValue>,
    },
    Refraction {
        strength: LengthValue,
        chromatic_aberration: LengthValue,
    },
    ChromaticAberration {
        offset_x: LengthValue,
        offset_y: LengthValue,
    },
}

impl BackdropFilter {
    pub fn blur(sigma: impl Into<LengthValue>) -> Self {
        let sigma = sigma.into();
        Self::Blur {
            sigma_x: sigma,
            sigma_y: sigma,
            quality: FilterQuality::Medium,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackdropStyle {
    pub filters: Arc<[BackdropFilter]>,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub mask: BackdropMask,
}

impl Default for BackdropStyle {
    fn default() -> Self {
        Self {
            filters: Arc::from([]),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: BackdropMask::None,
        }
    }
}

impl BackdropStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn blur(sigma: impl Into<LengthValue>) -> Self {
        Self {
            filters: Arc::from([BackdropFilter::blur(sigma)]),
            ..Self::default()
        }
    }

    pub fn with_filters(mut self, filters: impl Into<Arc<[BackdropFilter]>>) -> Self {
        self.filters = filters.into();
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    pub fn mask(mut self, mask: BackdropMask) -> Self {
        self.mask = mask;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Blur {
        sigma_x: LengthValue,
        sigma_y: LengthValue,
        quality: FilterQuality,
    },
    DropShadow {
        color: ColorValue,
        offset_x: LengthValue,
        offset_y: LengthValue,
        sigma_x: LengthValue,
        sigma_y: LengthValue,
        spread: LengthValue,
        quality: FilterQuality,
    },
    ColorMatrix(ColorMatrix),
    ImageMask {
        image: ImageKey,
        data: ImageData,
        bounds: Bounds,
    },
}

impl Effect {
    pub fn blur(sigma: impl Into<LengthValue>) -> Self {
        let sigma = sigma.into();
        Self::Blur {
            sigma_x: sigma,
            sigma_y: sigma,
            quality: FilterQuality::Medium,
        }
    }
}

impl From<f32> for LengthValue {
    fn from(value: f32) -> Self {
        Self::Px(value)
    }
}

impl From<SpacingToken> for LengthValue {
    fn from(value: SpacingToken) -> Self {
        Self::Spacing(value)
    }
}

impl From<RadiusToken> for LengthValue {
    fn from(value: RadiusToken) -> Self {
        Self::Radius(value)
    }
}

impl From<FontSizeToken> for LengthValue {
    fn from(value: FontSizeToken) -> Self {
        Self::FontSize(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexDirectionStyle {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScrollDirectionStyle {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

impl ScrollDirectionStyle {
    pub fn allows_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    pub fn allows_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    pub fn is_scrollable(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScrollbarVisibilityStyle {
    #[default]
    Auto,
    Always,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarStyle {
    pub width: LengthValue,
    pub track_color: ColorStyle,
    pub thumb_color: ColorStyle,
    pub radius: LengthValue,
    pub visibility: ScrollbarVisibilityStyle,
}

impl Default for ScrollbarStyle {
    fn default() -> Self {
        Self {
            width: LengthValue::Px(8.0),
            track_color: ColorStyle::Solid(Color::TRANSPARENT.into()),
            thumb_color: ColorStyle::Solid(Color::rgba(0.0, 0.0, 0.0, 0.35).into()),
            radius: LengthValue::Px(4.0),
            visibility: ScrollbarVisibilityStyle::Auto,
        }
    }
}

impl ScrollbarStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn width(mut self, width: impl Into<LengthValue>) -> Self {
        self.width = width.into();
        self
    }

    pub fn track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.track_color = color.into();
        self
    }

    pub fn thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.thumb_color = color.into();
        self
    }

    pub fn radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.radius = radius.into();
        self
    }

    pub fn visibility(mut self, visibility: ScrollbarVisibilityStyle) -> Self {
        self.visibility = visibility;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    pub color: ColorValue,
    pub offset: Point,
    pub blur: LengthValue,
    pub spread: LengthValue,
}

impl Default for ShadowStyle {
    fn default() -> Self {
        Self {
            color: Color::TRANSPARENT.into(),
            offset: Point::new(0.0, 0.0),
            blur: LengthValue::Px(0.0),
            spread: LengthValue::Px(0.0),
        }
    }
}

impl ShadowStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn color(mut self, color: impl Into<ColorValue>) -> Self {
        self.color = color.into();
        self
    }

    pub fn offset(mut self, offset: Point) -> Self {
        self.offset = offset;
        self
    }

    pub fn blur(mut self, blur: impl Into<LengthValue>) -> Self {
        self.blur = blur.into();
        self
    }

    pub fn spread(mut self, spread: impl Into<LengthValue>) -> Self {
        self.spread = spread.into();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignStyle {
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JustifyStyle {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PositionStyle {
    #[default]
    Relative,
    Absolute,
}

/// Whether a node takes part in layout at all.
///
/// `None` takes the node and its subtree out of layout, paint and hit
/// testing -- it occupies no space -- while leaving it mounted, so component
/// state survives. That is the difference from rendering it conditionally,
/// which unmounts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisplayStyle {
    #[default]
    Visible,
    None,
}

/// Whether a laid-out node is drawn.
///
/// `Hidden` keeps the node's space and its state but draws nothing and takes
/// no input, for the node and its whole subtree. It changes paint only, so
/// toggling it never re-runs layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VisibilityStyle {
    #[default]
    Visible,
    Hidden,
}

/// A visual transform applied after layout. `origin` is normalized within the
/// node's laid-out bounds, so `(0.5, 0.5)` is the center.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformStyle {
    pub translate: Point,
    pub scale: Point,
    /// Clockwise rotation in radians in XUI's y-down coordinate system.
    pub rotate: f32,
    pub origin: Point,
}

impl TransformStyle {
    pub const IDENTITY: Self = Self {
        translate: Point::zero(),
        scale: Point::new(1.0, 1.0),
        rotate: 0.0,
        origin: Point::new(0.5, 0.5),
    };

    pub const fn new() -> Self {
        Self::IDENTITY
    }

    pub const fn translate(mut self, translate: Point) -> Self {
        self.translate = translate;
        self
    }

    pub const fn scale(mut self, scale: Point) -> Self {
        self.scale = scale;
        self
    }

    pub const fn uniform_scale(mut self, scale: f32) -> Self {
        self.scale = Point::new(scale, scale);
        self
    }

    pub const fn rotate(mut self, radians: f32) -> Self {
        self.rotate = radians;
        self
    }

    pub const fn origin(mut self, origin: Point) -> Self {
        self.origin = origin;
        self
    }

    pub fn to_affine(self, size: Size<f32>) -> Affine {
        let origin = Point::new(self.origin.x * size.width, self.origin.y * size.height);
        let (sin, cos) = self.rotate.sin_cos();
        Affine::translate(-origin.x, -origin.y)
            .then(Affine::scale(self.scale.x, self.scale.y))
            .then(Affine::new(cos, sin, -sin, cos, 0.0, 0.0))
            .then(Affine::translate(
                origin.x + self.translate.x,
                origin.y + self.translate.y,
            ))
    }
}

impl Default for TransformStyle {
    fn default() -> Self {
        Self::IDENTITY
    }
}
