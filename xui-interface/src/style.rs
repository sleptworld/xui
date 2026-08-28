use crate::{
    core::Sizing, text::TextStyle, Affine, Bounds, Color, EdgeInsets, FontFamily, FontStyle,
    FontWeight, ImageData, ImageKey, LineHeight, Point, Size, StyleDiffFlags, TextDecoration,
    Transition, WidgetState,
};
use std::{
    cell::RefCell,
    fmt,
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut},
    sync::Arc,
};

thread_local! {
    static BASIC_STYLE: ComputedStyle = ComputedStyle::initial(&Theme::default());
}

/// Tokens
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorToken {
    Text,
    InverseText,
    Background,
    Surface,
    MutedSurface,
    Border,
    Primary,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontSizeToken {
    Sm,
    Md,
    Lg,
    Xl,
}

/// Styles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StyleValue<T> {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorValue {
    Color(Color),
    Token(ColorToken),
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComputedMaskShape {
    Rect,
    RoundedRect(f32),
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

#[derive(Debug, Clone, Copy, PartialEq, Hash, Default)]
pub struct ScrollbarStylePatch {
    pub width: StyleValue<LengthValue>,
    pub track_color: StyleValue<ColorStyle>,
    pub thumb_color: StyleValue<ColorStyle>,
    pub radius: StyleValue<LengthValue>,
    pub visibility: StyleValue<ScrollbarVisibilityStyle>,
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

/// Patches
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStylePatch {
    pub color: StyleValue<ColorValue>,
    pub font_family: StyleValue<FontFamily>,
    pub font_size: StyleValue<LengthValue>,
    pub font_weight: StyleValue<FontWeight>,
    pub font_style: StyleValue<FontStyle>,
    pub line_height: StyleValue<LineHeight>,
    pub letter_spacing: StyleValue<LengthValue>,
    pub decoration: StyleValue<TextDecoration>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutStylePatch {
    // pub flex_direction: StyleValue<FlexDirectionStyle>,
    pub gap: StyleValue<LengthValue>,
    pub width: StyleValue<Sizing>,
    pub height: StyleValue<Sizing>,
    pub min_height: StyleValue<Sizing>,
    pub max_height: StyleValue<Sizing>,
    pub min_width: StyleValue<Sizing>,
    pub max_width: StyleValue<Sizing>,
    pub margin: StyleValue<EdgeInsets>,
    pub padding: StyleValue<EdgeInsets>,
    pub align: StyleValue<AlignStyle>,
    pub justify: StyleValue<JustifyStyle>,
    pub position: StyleValue<PositionStyle>,
    pub inset: StyleValue<EdgeInsets>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintStylePatch {
    pub background: StyleValue<ColorStyle>,
    pub border_color: StyleValue<ColorStyle>,
    pub border_width: StyleValue<LengthValue>,
    pub border_radius: StyleValue<LengthValue>,
    pub stroke: StyleValue<StrokeStyle>,
    pub shadow: StyleValue<ShadowStyle>,
    pub clip: StyleValue<bool>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransformStylePatch {
    pub translate_x: StyleValue<f32>,
    pub translate_y: StyleValue<f32>,
    pub scale_x: StyleValue<f32>,
    pub scale_y: StyleValue<f32>,
    pub rotate: StyleValue<f32>,
    pub origin_x: StyleValue<f32>,
    pub origin_y: StyleValue<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectStylePatch {
    pub backdrop: StyleValue<BackdropStyle>,
    pub effects: StyleValue<Arc<[Effect]>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScrollStylePatch {
    pub direction: StyleValue<ScrollDirectionStyle>,
    pub scrollbar: ScrollbarStylePatch,
}

#[derive(Default)]
pub struct Style {
    pub base: StylePatch,
    pub rules: Vec<StateStyleRule>,
    transition: Option<Transition>,
    state_deps: WidgetState,
    patch_cache: RefCell<Vec<(WidgetState, StylePatch)>>,
}

impl fmt::Debug for Style {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Style")
            .field("base", &self.base)
            .field("rules", &self.rules)
            .field("transition", &self.transition)
            .field("state_deps", &self.state_deps)
            .finish()
    }
}

impl Clone for Style {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            rules: self.rules.clone(),
            transition: self.transition,
            state_deps: self.state_deps,
            patch_cache: RefCell::default(),
        }
    }
}

impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        self.base == other.base
            && self.rules == other.rules
            && self.transition == other.transition
            && self.state_deps == other.state_deps
    }
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_patch(base: StylePatch) -> Self {
        Self {
            base,
            rules: Vec::new(),
            transition: None,
            state_deps: WidgetState::empty(),
            patch_cache: RefCell::default(),
        }
    }

    pub fn merge<S: StyleMerge + ?Sized>(&mut self, other: &S) {
        other.merge_into(self);
    }

    pub fn transition(mut self, transition: Transition) -> Self {
        self.transition = Some(transition);
        self
    }

    pub fn clear_transition(mut self) -> Self {
        self.transition = None;
        self
    }

    pub fn transition_config(&self) -> Option<Transition> {
        self.transition
    }

    fn map_base(mut self, f: impl FnOnce(StylePatch) -> StylePatch) -> Self {
        self.base = f(self.base);
        self.patch_cache.borrow_mut().clear();
        self
    }

    pub fn cursor(self, cursor: CursorIcon) -> Self {
        self.map_base(|base| base.cursor(cursor))
    }

    pub fn color(self, color: impl Into<ColorValue>) -> Self {
        self.map_base(|base| base.color(color))
    }

    pub fn font_family(self, font_family: impl Into<FontFamily>) -> Self {
        self.map_base(|base| base.font_family(font_family))
    }

    pub fn font_size(self, font_size: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.font_size(font_size))
    }

    pub fn font_weight(self, font_weight: FontWeight) -> Self {
        self.map_base(|base| base.font_weight(font_weight))
    }

    pub fn font_style(self, font_style: FontStyle) -> Self {
        self.map_base(|base| base.font_style(font_style))
    }

    pub fn line_height(self, line_height: LineHeight) -> Self {
        self.map_base(|base| base.line_height(line_height))
    }

    pub fn letter_spacing(self, letter_spacing: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.letter_spacing(letter_spacing))
    }

    pub fn decoration(self, decoration: TextDecoration) -> Self {
        self.map_base(|base| base.decoration(decoration))
    }

    pub fn gap(self, gap: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.gap(gap))
    }

    pub fn size(self, size: impl Into<Size<Sizing>>) -> Self {
        self.map_base(|base| base.size(size))
    }

    pub fn width(self, width: impl Into<Sizing>) -> Self {
        self.map_base(|base| base.width(width))
    }

    pub fn height(self, height: impl Into<Sizing>) -> Self {
        self.map_base(|base| base.height(height))
    }

    pub fn min_size(self, size: impl Into<Size<Sizing>>) -> Self {
        self.map_base(|base| base.min_size(size))
    }

    pub fn min_width(self, width: impl Into<Sizing>) -> Self {
        self.map_base(|base| base.min_width(width))
    }

    pub fn min_height(self, height: impl Into<Sizing>) -> Self {
        self.map_base(|base| base.min_height(height))
    }

    pub fn max_size(self, size: Size<Sizing>) -> Self {
        self.map_base(|base| base.max_size(size))
    }

    pub fn max_width(self, width: impl Into<Sizing>) -> Self {
        self.map_base(|base| base.max_width(width))
    }

    pub fn max_height(self, height: impl Into<Sizing>) -> Self {
        self.map_base(|base| base.max_height(height))
    }

    pub fn margin(self, margin: EdgeInsets) -> Self {
        self.map_base(|base| base.margin(margin))
    }

    pub fn padding(self, padding: EdgeInsets) -> Self {
        self.map_base(|base| base.padding(padding))
    }

    pub fn align(self, align: AlignStyle) -> Self {
        self.map_base(|base| base.align(align))
    }

    pub fn justify(self, justify: JustifyStyle) -> Self {
        self.map_base(|base| base.justify(justify))
    }

    pub fn position_type(self, position: PositionStyle) -> Self {
        self.map_base(|base| base.position_type(position))
    }

    pub fn absolute(self) -> Self {
        self.position_type(PositionStyle::Absolute)
    }

    pub fn relative(self) -> Self {
        self.position_type(PositionStyle::Relative)
    }

    pub fn inset(self, inset: EdgeInsets) -> Self {
        self.map_base(|base| base.inset(inset))
    }

    pub fn transform(self, transform: TransformStyle) -> Self {
        self.map_base(|base| base.transform(transform))
    }

    pub fn translate(self, translate: Point) -> Self {
        self.map_base(|base| base.translate(translate))
    }

    pub fn translate_x(self, x: f32) -> Self {
        self.map_base(|base| base.translate_x(x))
    }

    pub fn translate_y(self, y: f32) -> Self {
        self.map_base(|base| base.translate_y(y))
    }

    pub fn scale(self, scale: f32) -> Self {
        self.map_base(|base| base.scale(scale))
    }

    pub fn scale_xy(self, scale: Point) -> Self {
        self.map_base(|base| base.scale_xy(scale))
    }

    pub fn rotate(self, radians: f32) -> Self {
        self.map_base(|base| base.rotate(radians))
    }

    pub fn transform_origin(self, origin: Point) -> Self {
        self.map_base(|base| base.transform_origin(origin))
    }

    pub fn background(self, color: impl Into<ColorStyle>) -> Self {
        self.map_base(|base| base.background(color))
    }

    pub fn border_color(self, color: impl Into<ColorStyle>) -> Self {
        self.map_base(|base| base.border_color(color))
    }

    pub fn border_width(self, width: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.border_width(width))
    }

    pub fn border_radius(self, radius: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.border_radius(radius))
    }

    pub fn stroke(self, stroke: StrokeStyle) -> Self {
        self.map_base(|base| base.stroke(stroke))
    }

    pub fn stroke_style(
        self,
        color: impl Into<ColorStyle>,
        width: impl Into<LengthValue>,
        line_style: StrokeLineStyle,
    ) -> Self {
        self.map_base(|base| base.stroke_style(color, width, line_style))
    }

    pub fn no_stroke(self) -> Self {
        self.map_base(StylePatch::no_stroke)
    }

    pub fn shadow(self, shadow: ShadowStyle) -> Self {
        self.map_base(|base| base.shadow(shadow))
    }

    pub fn box_shadow(
        self,
        color: impl Into<ColorValue>,
        offset: Point,
        blur: impl Into<LengthValue>,
        spread: impl Into<LengthValue>,
    ) -> Self {
        self.map_base(|base| base.box_shadow(color, offset, blur, spread))
    }

    pub fn no_shadow(self) -> Self {
        self.map_base(StylePatch::no_shadow)
    }

    pub fn clip(self, clip: bool) -> Self {
        self.map_base(|base| base.clip(clip))
    }

    /// Blurs the content already painted behind this widget.
    pub fn backdrop_blur(self, sigma: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.backdrop_blur(sigma))
    }

    pub fn backdrop_style(self, backdrop: BackdropStyle) -> Self {
        self.map_base(|base| base.backdrop_style(backdrop))
    }

    pub fn no_backdrop(self) -> Self {
        self.map_base(StylePatch::no_backdrop)
    }

    pub fn effects(self, effects: impl Into<Arc<[Effect]>>) -> Self {
        self.map_base(|base| base.effects(effects))
    }

    pub fn no_effects(self) -> Self {
        self.map_base(StylePatch::no_effects)
    }

    pub fn scroll_direction(self, direction: ScrollDirectionStyle) -> Self {
        self.map_base(|base| base.scroll_direction(direction))
    }

    pub fn scroll_vertical(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Vertical)
    }

    pub fn scroll_horizontal(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Horizontal)
    }

    pub fn scroll_both(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Both)
    }

    pub fn no_scroll(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::None)
    }

    pub fn scrollbar_style(self, scrollbar: ScrollbarStyle) -> Self {
        self.map_base(|base| base.scrollbar_style(scrollbar))
    }

    pub fn scrollbar(self, scrollbar: ScrollbarStylePatch) -> Self {
        self.map_base(|base| base.scrollbar(scrollbar))
    }

    pub fn scrollbar_width(self, width: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.scrollbar_width(width))
    }

    pub fn scrollbar_track_color(self, color: impl Into<ColorStyle>) -> Self {
        self.map_base(|base| base.scrollbar_track_color(color))
    }

    pub fn scrollbar_thumb_color(self, color: impl Into<ColorStyle>) -> Self {
        self.map_base(|base| base.scrollbar_thumb_color(color))
    }

    pub fn scrollbar_radius(self, radius: impl Into<LengthValue>) -> Self {
        self.map_base(|base| base.scrollbar_radius(radius))
    }

    pub fn scrollbar_visibility(self, visibility: ScrollbarVisibilityStyle) -> Self {
        self.map_base(|base| base.scrollbar_visibility(visibility))
    }

    pub fn when<F: FnOnce(StylePatch) -> StylePatch>(
        mut self,
        condition: WidgetState,
        f: F,
    ) -> Self {
        self = self.when_state(WidgetStateMatcher::all(condition), f);

        self
    }

    pub fn when_state<F: FnOnce(StylePatch) -> StylePatch>(
        mut self,
        matcher: WidgetStateMatcher,
        f: F,
    ) -> Self {
        let patch = f(StylePatch::default());
        self.rules.push(StateStyleRule { matcher, patch });
        self.state_deps |= matcher.dependencies();
        self.patch_cache.borrow_mut().clear();
        self
    }

    pub fn state_deps(&self) -> WidgetState {
        self.state_deps
    }

    pub fn match_state(&self, state: WidgetState) -> bool {
        for rule in &self.rules {
            if rule.matcher.matches(state) {
                return true;
            }
        }
        false
    }

    pub fn affects_state_change(&self, before: WidgetState, after: WidgetState) -> bool {
        if before == after {
            return false;
        }

        let changed = before ^ after;
        if !self.state_deps.intersects(changed) {
            return false;
        }

        for rule in &self.rules {
            if rule.matcher.matches_state_change(before, after) {
                return true;
            }
        }
        false
    }

    pub fn patch_for_state(&self, state: WidgetState) -> StylePatch {
        if let Some((_, patch)) = self
            .patch_cache
            .borrow()
            .iter()
            .find(|(cached_state, _)| *cached_state == state)
        {
            return patch.clone();
        }

        let mut patch = self.base.clone();
        for rule in &self.rules {
            if rule.matcher.matches(state) {
                patch.merge(&rule.patch);
            }
        }
        self.patch_cache.borrow_mut().push((state, patch.clone()));
        patch
    }
}

impl Deref for Style {
    type Target = StylePatch;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for Style {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.patch_cache.borrow_mut().clear();
        &mut self.base
    }
}

pub trait StyleMerge {
    fn merge_into(&self, target: &mut Style);
}

impl StyleMerge for Style {
    fn merge_into(&self, target: &mut Style) {
        target.base.merge(&self.base);
        target.rules.extend(self.rules.iter().cloned());
        if self.transition.is_some() {
            target.transition = self.transition;
        }
        target.state_deps |= self.state_deps;
        target.patch_cache.borrow_mut().clear();
    }
}

impl StyleMerge for StylePatch {
    fn merge_into(&self, target: &mut Style) {
        target.base.merge(self);
        target.patch_cache.borrow_mut().clear();
    }
}

/// The pointer shape a node asks the platform for.
///
/// A style property, so that it can be state-conditioned like any other —
/// `style!(cursor: if disabled { NotAllowed } else { Pointer })` — and so that
/// `cursor={..}` works on every widget without the DSL knowing about it.
///
/// It is deliberately absent from [`ComputedStyle::diff`]: a cursor produces no
/// scene output, so changing one must not invalidate layout, paint, or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorIcon {
    #[default]
    Default,
    /// The "this is clickable" hand.
    Pointer,
    /// An I-beam, for selectable text.
    Text,
    Crosshair,
    Move,
    /// Something can be picked up here.
    Grab,
    /// Something is being dragged right now.
    Grabbing,
    NotAllowed,
    Wait,
    Progress,
    Help,
    ColumnResize,
    RowResize,
    EastWestResize,
    NorthSouthResize,
    /// Draw nothing at all.
    None,
}

/// Style Patches Info
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StylePatch {
    pub text: TextStylePatch,
    pub layout: LayoutStylePatch,
    pub paint: PaintStylePatch,
    pub transform: TransformStylePatch,
    pub effect: EffectStylePatch,
    pub scroll: ScrollStylePatch,
    /// Not part of any group, because the groups are what `diff` compares and a
    /// cursor change must not dirty anything.
    pub cursor: StyleValue<CursorIcon>,
}

impl StylePatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(mut self, cursor: CursorIcon) -> Self {
        self.cursor = StyleValue::Value(cursor);
        self
    }

    pub fn color(mut self, color: impl Into<ColorValue>) -> Self {
        self.text.color = StyleValue::Value(color.into());
        self
    }

    pub fn font_family(mut self, font_family: impl Into<FontFamily>) -> Self {
        self.text.font_family = StyleValue::Value(font_family.into());
        self
    }

    pub fn font_size(mut self, font_size: impl Into<LengthValue>) -> Self {
        self.text.font_size = StyleValue::Value(font_size.into());
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.text.font_weight = StyleValue::Value(font_weight);
        self
    }

    pub fn font_style(mut self, font_style: FontStyle) -> Self {
        self.text.font_style = StyleValue::Value(font_style);
        self
    }

    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.text.line_height = StyleValue::Value(line_height);
        self
    }

    pub fn letter_spacing(mut self, letter_spacing: impl Into<LengthValue>) -> Self {
        self.text.letter_spacing = StyleValue::Value(letter_spacing.into());
        self
    }

    pub fn decoration(mut self, decoration: TextDecoration) -> Self {
        self.text.decoration = StyleValue::Value(decoration);
        self
    }

    pub fn gap(mut self, gap: impl Into<LengthValue>) -> Self {
        self.layout.gap = StyleValue::Value(gap.into());
        self
    }

    pub fn size(mut self, size: impl Into<Size<Sizing>>) -> Self {
        let size = size.into();
        self.layout.width = StyleValue::Value(size.width);
        self.layout.height = StyleValue::Value(size.height);
        self
    }

    pub fn width(mut self, width: impl Into<Sizing>) -> Self {
        self.layout.width = StyleValue::Value(width.into());
        self
    }

    pub fn height(mut self, height: impl Into<Sizing>) -> Self {
        self.layout.height = StyleValue::Value(height.into());
        self
    }

    pub fn min_size(mut self, size: impl Into<Size<Sizing>>) -> Self {
        let size = size.into();
        self.layout.min_height = StyleValue::Value(size.height);
        self.layout.min_width = StyleValue::Value(size.width);
        self
    }

    pub fn min_width(mut self, width: impl Into<Sizing>) -> Self {
        self.layout.min_width = StyleValue::Value(width.into());
        self
    }

    pub fn min_height(mut self, height: impl Into<Sizing>) -> Self {
        self.layout.min_height = StyleValue::Value(height.into());
        self
    }

    pub fn max_size(mut self, size: Size<Sizing>) -> Self {
        self.layout.max_height = StyleValue::Value(size.height);
        self.layout.max_width = StyleValue::Value(size.width);
        self
    }

    pub fn max_width(mut self, width: impl Into<Sizing>) -> Self {
        self.layout.max_width = StyleValue::Value(width.into());
        self
    }

    pub fn max_height(mut self, height: impl Into<Sizing>) -> Self {
        self.layout.max_height = StyleValue::Value(height.into());
        self
    }

    pub fn margin(mut self, margin: EdgeInsets) -> Self {
        self.layout.margin = StyleValue::Value(margin);
        self
    }

    pub fn padding(mut self, padding: EdgeInsets) -> Self {
        self.layout.padding = StyleValue::Value(padding);
        self
    }

    pub fn align(mut self, align: AlignStyle) -> Self {
        self.layout.align = StyleValue::Value(align);
        self
    }

    pub fn justify(mut self, justify: JustifyStyle) -> Self {
        self.layout.justify = StyleValue::Value(justify);
        self
    }

    pub fn position_type(mut self, position: PositionStyle) -> Self {
        self.layout.position = StyleValue::Value(position);
        self
    }

    pub fn absolute(self) -> Self {
        self.position_type(PositionStyle::Absolute)
    }

    pub fn relative(self) -> Self {
        self.position_type(PositionStyle::Relative)
    }

    pub fn inset(mut self, inset: EdgeInsets) -> Self {
        self.layout.inset = StyleValue::Value(inset);
        self
    }

    pub fn transform(mut self, transform: TransformStyle) -> Self {
        self.transform.translate_x = StyleValue::Value(transform.translate.x);
        self.transform.translate_y = StyleValue::Value(transform.translate.y);
        self.transform.scale_x = StyleValue::Value(transform.scale.x);
        self.transform.scale_y = StyleValue::Value(transform.scale.y);
        self.transform.rotate = StyleValue::Value(transform.rotate);
        self.transform.origin_x = StyleValue::Value(transform.origin.x);
        self.transform.origin_y = StyleValue::Value(transform.origin.y);
        self
    }

    pub fn translate(mut self, translate: Point) -> Self {
        self.transform.translate_x = StyleValue::Value(translate.x);
        self.transform.translate_y = StyleValue::Value(translate.y);
        self
    }

    pub fn translate_x(mut self, x: f32) -> Self {
        self.transform.translate_x = StyleValue::Value(x);
        self
    }

    pub fn translate_y(mut self, y: f32) -> Self {
        self.transform.translate_y = StyleValue::Value(y);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.transform.scale_x = StyleValue::Value(scale);
        self.transform.scale_y = StyleValue::Value(scale);
        self
    }

    pub fn scale_xy(mut self, scale: Point) -> Self {
        self.transform.scale_x = StyleValue::Value(scale.x);
        self.transform.scale_y = StyleValue::Value(scale.y);
        self
    }

    pub fn rotate(mut self, radians: f32) -> Self {
        self.transform.rotate = StyleValue::Value(radians);
        self
    }

    pub fn transform_origin(mut self, origin: Point) -> Self {
        self.transform.origin_x = StyleValue::Value(origin.x);
        self.transform.origin_y = StyleValue::Value(origin.y);
        self
    }

    pub fn background(mut self, color: impl Into<ColorStyle>) -> Self {
        self.paint.background = StyleValue::Value(color.into());
        self
    }

    pub fn border_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.paint.border_color = StyleValue::Value(color.into());
        self
    }

    pub fn border_width(mut self, width: impl Into<LengthValue>) -> Self {
        self.paint.border_width = StyleValue::Value(width.into());
        self
    }

    pub fn border_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.paint.border_radius = StyleValue::Value(radius.into());
        self
    }

    pub fn stroke(mut self, stroke: StrokeStyle) -> Self {
        self.paint.stroke = StyleValue::Value(stroke);
        self
    }

    pub fn stroke_style(
        mut self,
        color: impl Into<ColorStyle>,
        width: impl Into<LengthValue>,
        line_style: StrokeLineStyle,
    ) -> Self {
        self.paint.stroke =
            StyleValue::Value(StrokeStyle::new(color, width).line_style(line_style));
        self
    }

    pub fn no_stroke(mut self) -> Self {
        self.paint.stroke = StyleValue::Initial;
        self
    }

    pub fn shadow(mut self, shadow: ShadowStyle) -> Self {
        self.paint.shadow = StyleValue::Value(shadow);
        self
    }

    pub fn box_shadow(
        mut self,
        color: impl Into<ColorValue>,
        offset: Point,
        blur: impl Into<LengthValue>,
        spread: impl Into<LengthValue>,
    ) -> Self {
        self.paint.shadow = StyleValue::Value(
            ShadowStyle::new()
                .color(color)
                .offset(offset)
                .blur(blur)
                .spread(spread),
        );
        self
    }

    pub fn no_shadow(mut self) -> Self {
        self.paint.shadow = StyleValue::Initial;
        self
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.paint.clip = StyleValue::Value(clip);
        self
    }

    /// Blurs the content already painted behind this widget.
    pub fn backdrop_blur(mut self, sigma: impl Into<LengthValue>) -> Self {
        self.effect.backdrop = StyleValue::Value(BackdropStyle::blur(sigma));
        self
    }

    pub fn backdrop_style(mut self, backdrop: BackdropStyle) -> Self {
        self.effect.backdrop = StyleValue::Value(backdrop);
        self
    }

    pub fn no_backdrop(mut self) -> Self {
        self.effect.backdrop = StyleValue::Initial;
        self
    }

    pub fn effects(mut self, effects: impl Into<Arc<[Effect]>>) -> Self {
        self.effect.effects = StyleValue::Value(effects.into());
        self
    }

    pub fn no_effects(mut self) -> Self {
        self.effect.effects = StyleValue::Initial;
        self
    }

    pub fn scroll_direction(mut self, direction: ScrollDirectionStyle) -> Self {
        self.scroll.direction = StyleValue::Value(direction);
        self
    }

    pub fn scroll_vertical(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Vertical)
    }

    pub fn scroll_horizontal(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Horizontal)
    }

    pub fn scroll_both(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::Both)
    }

    pub fn no_scroll(self) -> Self {
        self.scroll_direction(ScrollDirectionStyle::None)
    }

    pub fn scrollbar_style(mut self, scrollbar: ScrollbarStyle) -> Self {
        self.scroll.scrollbar.width = StyleValue::Value(scrollbar.width);
        self.scroll.scrollbar.track_color = StyleValue::Value(scrollbar.track_color);
        self.scroll.scrollbar.thumb_color = StyleValue::Value(scrollbar.thumb_color);
        self.scroll.scrollbar.radius = StyleValue::Value(scrollbar.radius);
        self.scroll.scrollbar.visibility = StyleValue::Value(scrollbar.visibility);
        self
    }

    pub fn scrollbar(mut self, scrollbar: ScrollbarStylePatch) -> Self {
        self.scroll.scrollbar = scrollbar;
        self
    }

    pub fn scrollbar_width(mut self, width: impl Into<LengthValue>) -> Self {
        self.scroll.scrollbar.width = StyleValue::Value(width.into());
        self
    }

    pub fn scrollbar_track_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.scroll.scrollbar.track_color = StyleValue::Value(color.into());
        self
    }

    pub fn scrollbar_thumb_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.scroll.scrollbar.thumb_color = StyleValue::Value(color.into());
        self
    }

    pub fn scrollbar_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.scroll.scrollbar.radius = StyleValue::Value(radius.into());
        self
    }

    pub fn scrollbar_visibility(mut self, visibility: ScrollbarVisibilityStyle) -> Self {
        self.scroll.scrollbar.visibility = StyleValue::Value(visibility);
        self
    }

    pub fn merge(&mut self, other: &StylePatch) {
        merge_text(&mut self.text, &other.text);
        merge_layout(&mut self.layout, &other.layout);
        merge_paint(&mut self.paint, &other.paint);
        merge_transform(&mut self.transform, &other.transform);
        merge_effect(&mut self.effect, &other.effect);
        merge_scroll(&mut self.scroll, &other.scroll);
        merge_value(&mut self.cursor, &other.cursor);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetStateMatcher {
    required: WidgetState,
    forbidden: WidgetState,
}

impl WidgetStateMatcher {
    pub fn new(required: WidgetState, forbidden: WidgetState) -> Self {
        Self {
            required,
            forbidden,
        }
    }

    pub fn all(required: WidgetState) -> Self {
        Self::new(required, WidgetState::empty())
    }

    pub fn matches(&self, state: WidgetState) -> bool {
        state.contains(self.required) && !state.intersects(self.forbidden)
    }

    pub fn matches_state_change(&self, before: WidgetState, after: WidgetState) -> bool {
        self.matches(before) != self.matches(after)
    }

    pub fn dependencies(&self) -> WidgetState {
        self.required | self.forbidden
    }
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct StateStyleRule {
    matcher: WidgetStateMatcher,
    patch: StylePatch,
}

/// Computed Style
/// It's real
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedTextStyle {
    pub color: Color,
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing: f32,
    pub decoration: TextDecoration,
}

impl From<TextStyle> for ComputedTextStyle {
    fn from(style: TextStyle) -> Self {
        Self {
            color: style.color,
            font_family: style.font_family,
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: style.line_height,
            letter_spacing: style.letter_spacing,
            decoration: style.decoration,
        }
    }
}

impl From<&TextStyle> for ComputedTextStyle {
    fn from(style: &TextStyle) -> Self {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedLayoutStyle {
    pub flex_direction: FlexDirectionStyle,
    pub gap: f32,
    // pub size: Size<Sizing>,
    pub width: Sizing,
    pub height: Sizing,
    pub min_height: Option<Sizing>,
    pub max_height: Option<Sizing>,
    pub min_width: Option<Sizing>,
    pub max_width: Option<Sizing>,
    // pub min_size: Option<Size<Sizing>>,
    // pub max_size: Option<Size<Sizing>>,
    pub margin: EdgeInsets,
    pub padding: EdgeInsets,
    pub align: AlignStyle,
    pub justify: JustifyStyle,
    pub position: PositionStyle,
    /// Explicit positioning offsets. `None` maps to Taffy's `auto` inset on
    /// every edge, which preserves the intrinsic size of absolute children.
    pub inset: Option<EdgeInsets>,
}

impl ComputedLayoutStyle {
    pub fn size(&self) -> Size<Sizing> {
        Size::<Sizing>::new(self.width, self.height)
    }

    pub fn min_size(&self) -> Size<Option<Sizing>> {
        Size::new(self.min_width, self.min_height)
    }

    pub fn max_size(&self) -> Size<Option<Sizing>> {
        Size::new(self.max_width, self.max_height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedPaintStyle {
    pub background: ComputedColorStyle,
    pub border_color: ComputedColorStyle,
    pub border_width: f32,
    pub border_radius: f32,
    pub stroke: Option<ComputedStrokeStyle>,
    pub shadow: Option<ComputedShadowStyle>,
    pub clip: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComputedEffectStyle {
    pub backdrop: Option<ComputedBackdropStyle>,
    pub effects: Arc<[ComputedEffect]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedScrollStyle {
    pub direction: ScrollDirectionStyle,
    pub scrollbar: ComputedScrollbarStyle,
}

impl ComputedScrollStyle {
    pub fn is_scrollable(&self) -> bool {
        self.direction != ScrollDirectionStyle::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedScrollbarStyle {
    pub width: f32,
    pub track_color: ComputedColorStyle,
    pub thumb_color: ComputedColorStyle,
    pub radius: f32,
    pub visibility: ScrollbarVisibilityStyle,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub text: ComputedTextStyle,
    pub layout: ComputedLayoutStyle,
    pub paint: ComputedPaintStyle,
    pub transform: TransformStyle,
    pub effect: ComputedEffectStyle,
    pub scroll: ComputedScrollStyle,
    /// `None` means this node does not specify one; the resolver walks up to
    /// the nearest ancestor that does.
    ///
    /// Not inherited through [`ComputedStyle::inherited_from`] on purpose:
    /// inheriting would give every node a resolved copy to keep in step, while
    /// the walk happens only when the hover target changes.
    pub cursor: Option<CursorIcon>,
}

impl ComputedStyle {
    pub fn initial(theme: &Theme) -> Self {
        Self {
            text: ComputedTextStyle {
                color: theme.color(ColorToken::Text),
                font_family: FontFamily::System,
                font_size: theme.font_size(FontSizeToken::Md),
                font_weight: FontWeight::Normal,
                font_style: FontStyle::Normal,
                line_height: LineHeight::Normal,
                letter_spacing: 0.0,
                decoration: TextDecoration::default(),
            },
            layout: ComputedLayoutStyle {
                flex_direction: FlexDirectionStyle::Column,
                gap: 0.0,
                width: Sizing::Hug,
                height: Sizing::Hug,
                min_height: None,
                min_width: None,
                max_height: None,
                max_width: None,
                margin: EdgeInsets::zero(),
                padding: EdgeInsets::zero(),
                align: AlignStyle::Start,
                justify: JustifyStyle::Start,
                position: PositionStyle::Relative,
                inset: None,
            },
            paint: ComputedPaintStyle {
                background: ComputedColorStyle::Solid(Color::TRANSPARENT),
                border_color: ComputedColorStyle::Solid(Color::TRANSPARENT),
                border_width: 0.0,
                border_radius: 0.0,
                stroke: None,
                shadow: None,
                clip: false,
            },
            transform: TransformStyle::IDENTITY,
            effect: ComputedEffectStyle {
                backdrop: None,
                effects: Arc::from([]),
            },
            scroll: ComputedScrollStyle {
                direction: ScrollDirectionStyle::None,
                scrollbar: ComputedScrollbarStyle {
                    width: 8.0,
                    track_color: ComputedColorStyle::Solid(Color::TRANSPARENT),
                    thumb_color: ComputedColorStyle::Solid(Color::rgba(0.0, 0.0, 0.0, 0.35)),
                    radius: 4.0,
                    visibility: ScrollbarVisibilityStyle::Auto,
                },
            },
            cursor: None,
        }
    }

    pub fn compute(parent: &ComputedStyle, patch: &StylePatch, theme: &Theme) -> Self {
        let mut computed = parent.inherited_from(theme);
        computed.apply(parent, patch, theme);
        computed
    }

    pub fn apply(&mut self, parent: &ComputedStyle, patch: &StylePatch, theme: &Theme) {
        apply_text(&mut self.text, &parent.text, &patch.text, theme);
        apply_layout(&mut self.layout, &patch.layout, theme);
        apply_paint(&mut self.paint, &patch.paint, theme);
        apply_transform(&mut self.transform, &patch.transform);
        apply_effect(&mut self.effect, &patch.effect, theme);
        apply_scroll(&mut self.scroll, &patch.scroll, theme);
        match patch.cursor {
            StyleValue::Unset | StyleValue::Inherit => {}
            StyleValue::Initial => self.cursor = None,
            StyleValue::Value(cursor) => self.cursor = Some(cursor),
        }
    }

    pub fn diff(&self, other: &ComputedStyle) -> StyleDiffFlags {
        let mut flags = StyleDiffFlags::empty();

        if self.text != other.text {
            flags |= StyleDiffFlags::TEXT;
        }

        if self.layout != other.layout {
            flags |= StyleDiffFlags::LAYOUT;
        }

        if self.paint != other.paint {
            flags |= StyleDiffFlags::PAINT;
        }

        if self.transform != other.transform {
            flags |= StyleDiffFlags::TRANSFORM;
        }

        if self.effect != other.effect {
            flags |= StyleDiffFlags::EFFECT;
        }

        if self.scroll != other.scroll {
            flags |= StyleDiffFlags::SCROLL;
        }

        // `cursor` is intentionally not compared. It has no scene output, so a
        // change must not dirty layout, paint, or text; the platform layer picks
        // the current value up on its next pull. Adding it here would make
        // hovering a button repaint it.

        flags
    }

    pub fn inherited_from(&self, theme: &Theme) -> Self {
        let mut computed = Self::initial(theme);

        computed.text.color = self.text.color;
        computed.text.font_family = self.text.font_family.clone();
        computed.text.font_size = self.text.font_size;
        computed.text.font_weight = self.text.font_weight;
        computed.text.font_style = self.text.font_style;
        computed.text.line_height = self.text.line_height;
        computed.text.letter_spacing = self.text.letter_spacing;
        computed.text.decoration = self.text.decoration;

        computed
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub text: Color,
    pub inverse_text: Color,
    pub background: Color,
    pub surface: Color,
    pub muted_surface: Color,
    pub border: Color,
    pub primary: Color,
    pub spacing_xs: f32,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub spacing_xl: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub font_size_sm: f32,
    pub font_size_md: f32,
    pub font_size_lg: f32,
    pub font_size_xl: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            text: Color::BLACK,
            inverse_text: Color::WHITE,
            background: Color::WHITE,
            surface: Color::GRAY_100,
            muted_surface: Color::GRAY_300,
            border: Color::GRAY_300,
            primary: Color::BLUE_500,
            spacing_xs: 4.0,
            spacing_sm: 8.0,
            spacing_md: 12.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            radius_sm: 2.0,
            radius_md: 4.0,
            radius_lg: 8.0,
            font_size_sm: 12.0,
            font_size_md: 14.0,
            font_size_lg: 16.0,
            font_size_xl: 20.0,
        }
    }
}

impl Theme {
    pub fn color(&self, token: ColorToken) -> Color {
        match token {
            ColorToken::Text => self.text,
            ColorToken::InverseText => self.inverse_text,
            ColorToken::Background => self.background,
            ColorToken::Surface => self.surface,
            ColorToken::MutedSurface => self.muted_surface,
            ColorToken::Border => self.border,
            ColorToken::Primary => self.primary,
        }
    }

    pub fn spacing(&self, token: SpacingToken) -> f32 {
        match token {
            SpacingToken::Xs => self.spacing_xs,
            SpacingToken::Sm => self.spacing_sm,
            SpacingToken::Md => self.spacing_md,
            SpacingToken::Lg => self.spacing_lg,
            SpacingToken::Xl => self.spacing_xl,
        }
    }

    pub fn radius(&self, token: RadiusToken) -> f32 {
        match token {
            RadiusToken::Sm => self.radius_sm,
            RadiusToken::Md => self.radius_md,
            RadiusToken::Lg => self.radius_lg,
        }
    }

    pub fn font_size(&self, token: FontSizeToken) -> f32 {
        match token {
            FontSizeToken::Sm => self.font_size_sm,
            FontSizeToken::Md => self.font_size_md,
            FontSizeToken::Lg => self.font_size_lg,
            FontSizeToken::Xl => self.font_size_xl,
        }
    }
}

fn merge_text(target: &mut TextStylePatch, other: &TextStylePatch) {
    merge_value(&mut target.color, &other.color);
    merge_value(&mut target.font_family, &other.font_family);
    merge_value(&mut target.font_size, &other.font_size);
    merge_value(&mut target.font_weight, &other.font_weight);
    merge_value(&mut target.font_style, &other.font_style);
    merge_value(&mut target.line_height, &other.line_height);
    merge_value(&mut target.letter_spacing, &other.letter_spacing);
    merge_value(&mut target.decoration, &other.decoration);
}

fn merge_layout(target: &mut LayoutStylePatch, other: &LayoutStylePatch) {
    // merge_value(&mut target.flex_direction, &other.flex_direction);
    merge_value(&mut target.gap, &other.gap);
    merge_value(&mut target.width, &other.width);
    merge_value(&mut target.height, &other.height);
    merge_value(&mut target.min_height, &other.min_height);
    merge_value(&mut target.min_width, &other.min_width);
    merge_value(&mut target.max_height, &other.max_height);
    merge_value(&mut target.max_width, &other.max_width);
    merge_value(&mut target.margin, &other.margin);
    merge_value(&mut target.padding, &other.padding);
    merge_value(&mut target.align, &other.align);
    merge_value(&mut target.justify, &other.justify);
    merge_value(&mut target.position, &other.position);
    merge_value(&mut target.inset, &other.inset);
}

fn merge_paint(target: &mut PaintStylePatch, other: &PaintStylePatch) {
    merge_value(&mut target.background, &other.background);
    merge_value(&mut target.border_color, &other.border_color);
    merge_value(&mut target.border_width, &other.border_width);
    merge_value(&mut target.border_radius, &other.border_radius);
    merge_value(&mut target.stroke, &other.stroke);
    merge_value(&mut target.shadow, &other.shadow);
    merge_value(&mut target.clip, &other.clip);
}

fn merge_transform(target: &mut TransformStylePatch, other: &TransformStylePatch) {
    merge_value(&mut target.translate_x, &other.translate_x);
    merge_value(&mut target.translate_y, &other.translate_y);
    merge_value(&mut target.scale_x, &other.scale_x);
    merge_value(&mut target.scale_y, &other.scale_y);
    merge_value(&mut target.rotate, &other.rotate);
    merge_value(&mut target.origin_x, &other.origin_x);
    merge_value(&mut target.origin_y, &other.origin_y);
}

fn merge_effect(target: &mut EffectStylePatch, other: &EffectStylePatch) {
    merge_value(&mut target.backdrop, &other.backdrop);
    merge_value(&mut target.effects, &other.effects);
}

fn merge_scroll(target: &mut ScrollStylePatch, other: &ScrollStylePatch) {
    merge_value(&mut target.direction, &other.direction);
    merge_scrollbar(&mut target.scrollbar, &other.scrollbar);
}

fn merge_scrollbar(target: &mut ScrollbarStylePatch, other: &ScrollbarStylePatch) {
    merge_value(&mut target.width, &other.width);
    merge_value(&mut target.track_color, &other.track_color);
    merge_value(&mut target.thumb_color, &other.thumb_color);
    merge_value(&mut target.radius, &other.radius);
    merge_value(&mut target.visibility, &other.visibility);
}

fn merge_value<T: Clone>(target: &mut StyleValue<T>, other: &StyleValue<T>) {
    if !matches!(other, StyleValue::Unset) {
        *target = other.clone();
    }
}

fn apply_text(
    target: &mut ComputedTextStyle,
    parent: &ComputedTextStyle,
    patch: &TextStylePatch,
    theme: &Theme,
) {
    target.color = resolve_color(
        patch.color,
        target.color,
        parent.color,
        theme.color(ColorToken::Text),
        theme,
    );
    target.font_family = resolve_clone(
        &patch.font_family,
        &target.font_family,
        &parent.font_family,
        &FontFamily::System,
    );
    target.font_size = resolve_length(
        patch.font_size,
        target.font_size,
        parent.font_size,
        theme.font_size(FontSizeToken::Md),
        theme,
    );
    target.font_weight = resolve_copy(
        patch.font_weight,
        target.font_weight,
        parent.font_weight,
        FontWeight::Normal,
    );
    target.font_style = resolve_copy(
        patch.font_style,
        target.font_style,
        parent.font_style,
        FontStyle::Normal,
    );
    target.line_height = resolve_copy(
        patch.line_height,
        target.line_height,
        parent.line_height,
        LineHeight::Normal,
    );
    target.letter_spacing = resolve_length(
        patch.letter_spacing,
        target.letter_spacing,
        parent.letter_spacing,
        0.0,
        theme,
    );
    target.decoration = resolve_copy(
        patch.decoration,
        target.decoration,
        parent.decoration,
        TextDecoration::default(),
    );
}

fn apply_layout(target: &mut ComputedLayoutStyle, patch: &LayoutStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).layout;

    target.gap = resolve_length_no_inherit(patch.gap, target.gap, initial.gap, theme);
    target.width = resolve_copy_no_inherit(patch.width, target.width, initial.width);
    target.height = resolve_copy_no_inherit(patch.height, target.height, initial.height);

    target.min_height =
        resolve_optional_copy_no_inherit(patch.min_height, target.min_height, initial.min_height);
    target.min_width =
        resolve_optional_copy_no_inherit(patch.min_width, target.min_width, initial.min_width);

    target.max_height =
        resolve_optional_copy_no_inherit(patch.max_height, target.max_height, initial.max_height);
    target.max_width =
        resolve_optional_copy_no_inherit(patch.max_width, target.max_width, initial.max_width);
    target.margin = resolve_copy_no_inherit(patch.margin, target.margin, initial.margin);
    target.padding = resolve_copy_no_inherit(patch.padding, target.padding, initial.padding);
    target.align = resolve_copy_no_inherit(patch.align, target.align, initial.align);
    target.justify = resolve_copy_no_inherit(patch.justify, target.justify, initial.justify);
    target.position = resolve_copy_no_inherit(patch.position, target.position, initial.position);
    target.inset = resolve_optional_copy_no_inherit(patch.inset, target.inset, initial.inset);
}

fn apply_paint(target: &mut ComputedPaintStyle, patch: &PaintStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).paint;
    target.background = resolve_color_style_no_inherit(
        patch.background,
        target.background,
        initial.background,
        theme,
    );
    target.border_color = resolve_color_style_no_inherit(
        patch.border_color,
        target.border_color,
        initial.border_color,
        theme,
    );
    target.border_width = resolve_length_no_inherit(
        patch.border_width,
        target.border_width,
        initial.border_width,
        theme,
    );
    target.border_radius = resolve_length_no_inherit(
        patch.border_radius,
        target.border_radius,
        initial.border_radius,
        theme,
    );
    target.stroke = resolve_stroke_no_inherit(
        patch.stroke,
        target.stroke,
        initial.stroke,
        target.border_color,
        target.border_width,
        theme,
    );
    target.shadow = resolve_shadow_no_inherit(patch.shadow, target.shadow, initial.shadow, theme);
    target.clip = resolve_copy_no_inherit(patch.clip, target.clip, initial.clip);
}

fn apply_transform(target: &mut TransformStyle, patch: &TransformStylePatch) {
    let initial = TransformStyle::IDENTITY;
    target.translate.x =
        resolve_copy_no_inherit(patch.translate_x, target.translate.x, initial.translate.x);
    target.translate.y =
        resolve_copy_no_inherit(patch.translate_y, target.translate.y, initial.translate.y);
    target.scale.x = resolve_copy_no_inherit(patch.scale_x, target.scale.x, initial.scale.x);
    target.scale.y = resolve_copy_no_inherit(patch.scale_y, target.scale.y, initial.scale.y);
    target.rotate = resolve_copy_no_inherit(patch.rotate, target.rotate, initial.rotate);
    target.origin.x = resolve_copy_no_inherit(patch.origin_x, target.origin.x, initial.origin.x);
    target.origin.y = resolve_copy_no_inherit(patch.origin_y, target.origin.y, initial.origin.y);
}

fn apply_effect(target: &mut ComputedEffectStyle, patch: &EffectStylePatch, theme: &Theme) {
    match &patch.backdrop {
        StyleValue::Unset | StyleValue::Inherit => {}
        StyleValue::Initial => target.backdrop = None,
        StyleValue::Value(backdrop) => {
            target.backdrop = Some(compute_backdrop_style(backdrop, theme));
        }
    }

    match &patch.effects {
        StyleValue::Unset | StyleValue::Inherit => {}
        StyleValue::Initial => target.effects = Arc::from([]),
        StyleValue::Value(effects) => {
            target.effects = effects
                .iter()
                .map(|effect| compute_effect(effect, theme))
                .collect::<Vec<_>>()
                .into();
        }
    }
}

fn compute_backdrop_style(style: &BackdropStyle, theme: &Theme) -> ComputedBackdropStyle {
    ComputedBackdropStyle {
        filters: style
            .filters
            .iter()
            .map(|filter| compute_backdrop_filter(filter, theme))
            .collect::<Vec<_>>()
            .into(),
        opacity: style.opacity.clamp(0.0, 1.0),
        blend_mode: style.blend_mode,
        mask: compute_backdrop_mask(&style.mask, theme),
    }
}

fn compute_backdrop_filter(filter: &BackdropFilter, theme: &Theme) -> ComputedBackdropFilter {
    match filter {
        BackdropFilter::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => ComputedBackdropFilter::Blur {
            sigma_x: non_negative(length_value(*sigma_x, theme)),
            sigma_y: non_negative(length_value(*sigma_y, theme)),
            quality: *quality,
        },
        BackdropFilter::Saturate(value) => ComputedBackdropFilter::Saturate(non_negative(*value)),
        BackdropFilter::Brightness(value) => {
            ComputedBackdropFilter::Brightness(non_negative(*value))
        }
        BackdropFilter::Contrast(value) => ComputedBackdropFilter::Contrast(non_negative(*value)),
        BackdropFilter::Grayscale(value) => {
            ComputedBackdropFilter::Grayscale(value.clamp(0.0, 1.0))
        }
        BackdropFilter::Sepia(value) => ComputedBackdropFilter::Sepia(value.clamp(0.0, 1.0)),
        BackdropFilter::HueRotate(value) => {
            ComputedBackdropFilter::HueRotate(normalize_radians(*value))
        }
        BackdropFilter::Invert(value) => ComputedBackdropFilter::Invert(value.clamp(0.0, 1.0)),
        BackdropFilter::ColorMatrix(matrix) => ComputedBackdropFilter::ColorMatrix(*matrix),
        BackdropFilter::Pixelate { size } => ComputedBackdropFilter::Pixelate {
            size: Size::new(
                non_negative(length_value(size.width, theme)),
                non_negative(length_value(size.height, theme)),
            ),
        },
        BackdropFilter::Refraction {
            strength,
            chromatic_aberration,
        } => ComputedBackdropFilter::Refraction {
            strength: length_value(*strength, theme),
            chromatic_aberration: length_value(*chromatic_aberration, theme).abs(),
        },
        BackdropFilter::ChromaticAberration { offset_x, offset_y } => {
            ComputedBackdropFilter::ChromaticAberration {
                offset: [
                    length_value(*offset_x, theme),
                    length_value(*offset_y, theme),
                ],
            }
        }
    }
}

fn compute_backdrop_mask(mask: &BackdropMask, theme: &Theme) -> ComputedBackdropMask {
    match mask {
        BackdropMask::None => ComputedBackdropMask::None,
        BackdropMask::Shape { shape, transform } => ComputedBackdropMask::Shape {
            shape: match shape {
                MaskShape::Rect => ComputedMaskShape::Rect,
                MaskShape::RoundedRect(radius) => {
                    ComputedMaskShape::RoundedRect(non_negative(length_value(*radius, theme)))
                }
                MaskShape::Circle => ComputedMaskShape::Circle,
                MaskShape::Ellipse => ComputedMaskShape::Ellipse,
                MaskShape::Line { from, to } => ComputedMaskShape::Line {
                    from: *from,
                    to: *to,
                },
            },
            transform: *transform,
        },
        BackdropMask::AlphaTexture { texture, transform } => ComputedBackdropMask::AlphaTexture {
            texture: texture.clone(),
            transform: *transform,
        },
    }
}

fn compute_effect(effect: &Effect, theme: &Theme) -> ComputedEffect {
    match effect {
        Effect::Blur {
            sigma_x,
            sigma_y,
            quality,
        } => ComputedEffect::Blur {
            sigma_x: non_negative(length_value(*sigma_x, theme)),
            sigma_y: non_negative(length_value(*sigma_y, theme)),
            quality: *quality,
        },
        Effect::DropShadow {
            color,
            offset_x,
            offset_y,
            sigma_x,
            sigma_y,
            spread,
            quality,
        } => ComputedEffect::DropShadow {
            color: color_value(*color, theme),
            offset: Point::new(
                length_value(*offset_x, theme),
                length_value(*offset_y, theme),
            ),
            sigma_x: non_negative(length_value(*sigma_x, theme)),
            sigma_y: non_negative(length_value(*sigma_y, theme)),
            spread: non_negative(length_value(*spread, theme)),
            quality: *quality,
        },
        Effect::ColorMatrix(matrix) => ComputedEffect::ColorMatrix(*matrix),
        Effect::ImageMask {
            image,
            data,
            bounds,
        } => ComputedEffect::ImageMask {
            image: image.clone(),
            data: data.clone(),
            bounds: *bounds,
        },
    }
}

fn non_negative(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else {
        value
    }
}

fn normalize_radians(value: f32) -> f32 {
    if value.is_finite() {
        let normalized =
            (value + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if normalized.abs() <= 1.0e-6 {
            0.0
        } else {
            normalized
        }
    } else {
        value
    }
}

fn apply_scroll(target: &mut ComputedScrollStyle, patch: &ScrollStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).scroll;
    target.direction =
        resolve_copy_no_inherit(patch.direction, target.direction, initial.direction);
    target.scrollbar =
        resolve_scrollbar_no_inherit(patch.scrollbar, target.scrollbar, initial.scrollbar, theme);
}

fn resolve_scrollbar_no_inherit(
    value: ScrollbarStylePatch,
    current: ComputedScrollbarStyle,
    initial: ComputedScrollbarStyle,
    theme: &Theme,
) -> ComputedScrollbarStyle {
    let width = resolve_length_no_inherit(value.width, current.width, initial.width, theme);
    let track_color = resolve_color_style_no_inherit(
        value.track_color,
        current.track_color,
        initial.track_color,
        theme,
    );
    let thumb_color = resolve_color_style_no_inherit(
        value.thumb_color,
        current.thumb_color,
        initial.thumb_color,
        theme,
    );
    let radius = resolve_length_no_inherit(value.radius, current.radius, initial.radius, theme);
    let visibility =
        resolve_copy_no_inherit(value.visibility, current.visibility, initial.visibility);
    ComputedScrollbarStyle {
        width,
        track_color,
        thumb_color,
        radius,
        visibility,
    }
}

fn resolve_stroke_no_inherit(
    value: StyleValue<StrokeStyle>,
    current: Option<ComputedStrokeStyle>,
    initial: Option<ComputedStrokeStyle>,
    border_color: ComputedColorStyle,
    border_width: f32,
    theme: &Theme,
) -> Option<ComputedStrokeStyle> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => {
            if border_width > 0.0 && border_color.is_visible() {
                Some(ComputedStrokeStyle {
                    color: border_color,
                    width: border_width,
                    line_style: StrokeLineStyle::Solid,
                })
            } else {
                current
            }
        }
        StyleValue::Initial => initial,
        StyleValue::Value(value) => {
            let color = color_style(value.color, theme);
            (length_value(value.width, theme) > 0.0 && color.is_visible()).then_some(
                ComputedStrokeStyle {
                    color,
                    width: length_value(value.width, theme),
                    line_style: value.line_style,
                },
            )
        }
    }
}

fn resolve_shadow_no_inherit(
    value: StyleValue<ShadowStyle>,
    current: Option<ComputedShadowStyle>,
    initial: Option<ComputedShadowStyle>,
    theme: &Theme,
) -> Option<ComputedShadowStyle> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => Some(ComputedShadowStyle {
            color: color_value(value.color, theme),
            offset: value.offset,
            blur: length_value(value.blur, theme),
            spread: length_value(value.spread, theme),
        }),
    }
}

fn resolve_color(
    value: StyleValue<ColorValue>,
    current: Color,
    inherited: Color,
    initial: Color,
    theme: &Theme,
) -> Color {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => color_value(value, theme),
    }
}

fn resolve_color_style_no_inherit(
    value: StyleValue<ColorStyle>,
    current: ComputedColorStyle,
    initial: ComputedColorStyle,
    theme: &Theme,
) -> ComputedColorStyle {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => color_style(value, theme),
    }
}

fn color_value(value: ColorValue, theme: &Theme) -> Color {
    match value {
        ColorValue::Color(color) => color,
        ColorValue::Token(token) => theme.color(token),
    }
}

fn color_style(value: ColorStyle, theme: &Theme) -> ComputedColorStyle {
    match value {
        ColorStyle::Solid(color) => ComputedColorStyle::Solid(color_value(color, theme)),
        ColorStyle::LinearGradient(gradient) => {
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: gradient.start,
                end: gradient.end,
                from: color_value(gradient.from, theme),
                to: color_value(gradient.to, theme),
            })
        }
        ColorStyle::RadialGradient(gradient) => {
            ComputedColorStyle::RadialGradient(ComputedRadialGradientStyle {
                center: gradient.center,
                radius: length_value(gradient.radius, theme),
                from: color_value(gradient.from, theme),
                to: color_value(gradient.to, theme),
            })
        }
    }
}

fn resolve_length(
    value: StyleValue<LengthValue>,
    current: f32,
    inherited: f32,
    initial: f32,
    theme: &Theme,
) -> f32 {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => length_value(value, theme),
    }
}

fn resolve_length_no_inherit(
    value: StyleValue<LengthValue>,
    current: f32,
    initial: f32,
    theme: &Theme,
) -> f32 {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => length_value(value, theme),
    }
}

fn length_value(value: LengthValue, theme: &Theme) -> f32 {
    match value {
        LengthValue::Px(value) => value,
        LengthValue::Spacing(token) => theme.spacing(token),
        LengthValue::Radius(token) => theme.radius(token),
        LengthValue::FontSize(token) => theme.font_size(token),
    }
}

fn resolve_copy<T: Copy>(value: StyleValue<T>, current: T, inherited: T, initial: T) -> T {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => value,
    }
}

fn resolve_clone<T: Clone>(value: &StyleValue<T>, current: &T, inherited: &T, initial: &T) -> T {
    match value {
        StyleValue::Unset => current.clone(),
        StyleValue::Inherit => inherited.clone(),
        StyleValue::Initial => initial.clone(),
        StyleValue::Value(value) => value.clone(),
    }
}

fn resolve_copy_no_inherit<T: Copy>(value: StyleValue<T>, current: T, initial: T) -> T {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => value,
    }
}

fn resolve_optional_copy_no_inherit<T: Copy>(
    value: StyleValue<T>,
    current: Option<T>,
    initial: Option<T>,
) -> Option<T> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => Some(value),
    }
}

fn resolve_optional_size_no_inherit(
    value: StyleValue<Size<Sizing>>,
    current: Option<Size<Sizing>>,
    initial: Option<Size<Sizing>>,
) -> Option<Size<Sizing>> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => Some(value),
    }
}

fn hash_color<H: Hasher>(color: Color, state: &mut H) {
    color.r.to_bits().hash(state);
    color.g.to_bits().hash(state);
    color.b.to_bits().hash(state);
    color.a.to_bits().hash(state);
}

fn hash_f32<H: Hasher>(value: f32, state: &mut H) {
    let bits = if value == 0.0 {
        0.0f32.to_bits()
    } else {
        value.to_bits()
    };
    bits.hash(state);
}

fn hash_affine<H: Hasher>(value: Affine, state: &mut H) {
    for component in [value.xx, value.yx, value.xy, value.yy, value.dx, value.dy] {
        hash_f32(component, state);
    }
}

fn hash_rect<H: Hasher>(value: Bounds, state: &mut H) {
    for component in [value.min.x, value.min.y, value.max.x, value.max.y] {
        hash_f32(component, state);
    }
}

fn hash_color_matrix<H: Hasher>(matrix: &ColorMatrix, state: &mut H) {
    for value in matrix {
        hash_f32(*value, state);
    }
}

fn hash_edge_insets<H: Hasher>(value: EdgeInsets, state: &mut H) {
    value.left.to_bits().hash(state);
    value.right.to_bits().hash(state);
    value.top.to_bits().hash(state);
    value.bottom.to_bits().hash(state);
}

fn hash_size<H: Hasher>(value: Size<Sizing>, state: &mut H) {
    value.width.hash(state);
    value.height.hash(state);
}

fn hash_point<H: Hasher>(value: Point, state: &mut H) {
    hash_f32(value.x, state);
    hash_f32(value.y, state);
}

impl Hash for ColorValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Color(color) => hash_color(*color, state),
            Self::Token(token) => token.hash(state),
        }
    }
}

impl Hash for ColorStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Solid(color) => color.hash(state),
            Self::LinearGradient(gradient) => gradient.hash(state),
            Self::RadialGradient(gradient) => gradient.hash(state),
        }
    }
}

impl Hash for LinearGradientStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_point(self.start, state);
        hash_point(self.end, state);
        self.from.hash(state);
        self.to.hash(state);
    }
}

impl Hash for RadialGradientStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_point(self.center, state);
        self.radius.hash(state);
        self.from.hash(state);
        self.to.hash(state);
    }
}

impl Hash for LengthValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Px(value) => value.to_bits().hash(state),
            Self::Spacing(token) => token.hash(state),
            Self::Radius(token) => token.hash(state),
            Self::FontSize(token) => token.hash(state),
        }
    }
}

impl Hash for MaskShape {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::RoundedRect(radius) => radius.hash(state),
            Self::Line { from, to } => {
                hash_point(*from, state);
                hash_point(*to, state);
            }
            Self::Rect | Self::Circle | Self::Ellipse => {}
        }
    }
}

impl Hash for BackdropMask {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::None => {}
            Self::Shape { shape, transform } => {
                shape.hash(state);
                hash_affine(*transform, state);
            }
            Self::AlphaTexture { texture, transform } => {
                texture.hash(state);
                hash_affine(*transform, state);
            }
        }
    }
}

impl Hash for BackdropFilter {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Blur {
                sigma_x,
                sigma_y,
                quality,
            } => {
                sigma_x.hash(state);
                sigma_y.hash(state);
                quality.hash(state);
            }
            Self::Saturate(value)
            | Self::Brightness(value)
            | Self::Contrast(value)
            | Self::Grayscale(value)
            | Self::Sepia(value)
            | Self::HueRotate(value)
            | Self::Invert(value) => hash_f32(*value, state),
            Self::ColorMatrix(matrix) => hash_color_matrix(matrix, state),
            Self::Pixelate { size } => {
                size.width.hash(state);
                size.height.hash(state);
            }
            Self::Refraction {
                strength,
                chromatic_aberration,
            } => {
                strength.hash(state);
                chromatic_aberration.hash(state);
            }
            Self::ChromaticAberration { offset_x, offset_y } => {
                offset_x.hash(state);
                offset_y.hash(state);
            }
        }
    }
}

impl Hash for BackdropStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.filters.hash(state);
        hash_f32(self.opacity, state);
        self.blend_mode.hash(state);
        self.mask.hash(state);
    }
}

impl Hash for Effect {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Blur {
                sigma_x,
                sigma_y,
                quality,
            } => {
                sigma_x.hash(state);
                sigma_y.hash(state);
                quality.hash(state);
            }
            Self::DropShadow {
                color,
                offset_x,
                offset_y,
                sigma_x,
                sigma_y,
                spread,
                quality,
            } => {
                color.hash(state);
                offset_x.hash(state);
                offset_y.hash(state);
                sigma_x.hash(state);
                sigma_y.hash(state);
                spread.hash(state);
                quality.hash(state);
            }
            Self::ColorMatrix(matrix) => hash_color_matrix(matrix, state),
            Self::ImageMask {
                image,
                data,
                bounds,
            } => {
                image.hash(state);
                data.id().hash(state);
                hash_rect(*bounds, state);
            }
        }
    }
}

impl Hash for Style {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.base.hash(state);
        self.rules.hash(state);
        self.transition.hash(state);
    }
}

impl Hash for StylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.layout.hash(state);
        self.paint.hash(state);
        self.transform.hash(state);
        self.effect.hash(state);
        self.scroll.hash(state);
    }
}

impl Hash for TextStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        self.font_family.hash(state);
        self.font_size.hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.line_height.hash(state);
        self.letter_spacing.hash(state);
        self.decoration.hash(state);
    }
}

impl Hash for LayoutStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.gap.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.min_height.hash(state);
        self.min_width.hash(state);
        self.max_height.hash(state);
        self.max_width.hash(state);
        self.align.hash(state);
        self.justify.hash(state);
        self.position.hash(state);
        hash_style_value_edge_insets(&self.margin, state);
        hash_style_value_edge_insets(&self.padding, state);
        hash_style_value_edge_insets(&self.inset, state);
    }
}

impl Hash for PaintStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.background.hash(state);
        self.border_color.hash(state);
        self.border_width.hash(state);
        self.border_radius.hash(state);
        hash_style_value_stroke(&self.stroke, state);
        hash_style_value_shadow(&self.shadow, state);
        self.clip.hash(state);
    }
}

impl Hash for TransformStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_style_value_f32(&self.translate_x, state);
        hash_style_value_f32(&self.translate_y, state);
        hash_style_value_f32(&self.scale_x, state);
        hash_style_value_f32(&self.scale_y, state);
        hash_style_value_f32(&self.rotate, state);
        hash_style_value_f32(&self.origin_x, state);
        hash_style_value_f32(&self.origin_y, state);
    }
}

impl Hash for EffectStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_style_value_backdrop(&self.backdrop, state);
        hash_style_value_effects(&self.effects, state);
    }
}

impl Hash for ScrollStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.direction.hash(state);
        hash_style_value_scrollbar(&self.scrollbar, state);
    }
}

fn hash_style_value_size<H: Hasher>(value: &StyleValue<Size<Sizing>>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        hash_size(*value, state);
    }
}

fn hash_style_value_edge_insets<H: Hasher>(value: &StyleValue<EdgeInsets>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        hash_edge_insets(*value, state);
    }
}

fn hash_style_value_f32<H: Hasher>(value: &StyleValue<f32>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.to_bits().hash(state);
    }
}

fn hash_style_value_backdrop<H: Hasher>(value: &StyleValue<BackdropStyle>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.hash(state);
    }
}

fn hash_style_value_effects<H: Hasher>(value: &StyleValue<Arc<[Effect]>>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.hash(state);
    }
}

fn hash_style_value_stroke<H: Hasher>(value: &StyleValue<StrokeStyle>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.color.hash(state);
        value.width.hash(state);
        value.line_style.hash(state);
    }
}

fn hash_style_value_shadow<H: Hasher>(value: &StyleValue<ShadowStyle>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        value.color.hash(state);
        hash_point(value.offset, state);
        value.blur.hash(state);
        value.spread.hash(state);
    }
}

fn hash_style_value_scrollbar<H: Hasher>(value: &ScrollbarStylePatch, state: &mut H) {
    value.width.hash(state);
    value.track_color.hash(state);
    value.thumb_color.hash(state);
    value.radius.hash(state);
    value.visibility.hash(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::time::Duration;

    #[test]
    fn backdrop_defaults_to_an_empty_unmasked_style() {
        let authored = BackdropStyle::default();
        let computed = ComputedBackdropStyle::default();

        assert!(authored.filters.is_empty());
        assert_eq!(authored.opacity, 1.0);
        assert_eq!(authored.blend_mode, BlendMode::Normal);
        assert_eq!(authored.mask, BackdropMask::None);
        assert!(computed.filters.is_empty());
        assert_eq!(computed.opacity, 1.0);
        assert_eq!(computed.mask, ComputedBackdropMask::None);
        assert!(ComputedEffectStyle::default().effects.is_empty());
    }

    #[test]
    fn computed_style_diff_is_empty_when_styles_match() {
        let theme = Theme::default();
        let style = ComputedStyle::initial(&theme);

        assert!(style.diff(&style).is_empty());
    }

    #[test]
    fn computed_style_diff_marks_paint_changes() {
        let theme = Theme::default();
        let current = ComputedStyle::initial(&theme);
        let mut next = current.clone();
        next.paint.background = ComputedColorStyle::Solid(Color::BLACK);

        let flags = current.diff(&next);

        assert!(flags.contains(StyleDiffFlags::PAINT));
        assert!(!flags.contains(StyleDiffFlags::LAYOUT));
        assert!(!flags.contains(StyleDiffFlags::TEXT));
    }

    #[test]
    fn backdrop_blur_defaults_to_zero_clamps_and_does_not_inherit() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let parent =
            ComputedStyle::compute(&initial, &StylePatch::new().backdrop_blur(18.0), &theme);
        let child = ComputedStyle::compute(&parent, &StylePatch::new(), &theme);
        let clamped =
            ComputedStyle::compute(&initial, &StylePatch::new().backdrop_blur(-4.0), &theme);

        let blur_sigma = |style: &ComputedStyle| {
            style
                .effect
                .backdrop
                .as_ref()
                .and_then(|backdrop| backdrop.filters.first())
                .and_then(|filter| match filter {
                    ComputedBackdropFilter::Blur { sigma_x, .. } => Some(*sigma_x),
                    _ => None,
                })
        };

        assert_eq!(initial.effect.backdrop, None);
        assert_eq!(blur_sigma(&parent), Some(18.0));
        assert_eq!(child.effect.backdrop, None);
        assert_eq!(blur_sigma(&clamped), Some(0.0));
    }

    #[test]
    fn backdrop_blur_diff_is_effect_only() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let blurred =
            ComputedStyle::compute(&initial, &StylePatch::new().backdrop_blur(12.0), &theme);

        assert_eq!(initial.diff(&blurred), StyleDiffFlags::EFFECT);
    }

    #[test]
    fn backdrop_blur_participates_in_state_merge_and_hash() {
        let style = Style::new()
            .backdrop_blur(4.0)
            .when(WidgetState::HOVERED, |style| style.backdrop_blur(16.0));
        let normal = style.patch_for_state(WidgetState::empty());
        let hovered = style.patch_for_state(WidgetState::HOVERED);

        assert_eq!(
            normal.effect.backdrop,
            StyleValue::Value(BackdropStyle::blur(4.0))
        );
        assert_eq!(
            hovered.effect.backdrop,
            StyleValue::Value(BackdropStyle::blur(16.0))
        );

        let hash = |patch: &StylePatch| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            patch.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash(&normal), hash(&hovered));
    }

    #[test]
    fn backdrop_blur_shortcut_matches_full_backdrop_style() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let shortcut =
            ComputedStyle::compute(&initial, &StylePatch::new().backdrop_blur(12.0), &theme);
        let full = ComputedStyle::compute(
            &initial,
            &StylePatch::new().backdrop_style(BackdropStyle::new().with_filters(Arc::from([
                BackdropFilter::Blur {
                    sigma_x: LengthValue::Px(12.0),
                    sigma_y: LengthValue::Px(12.0),
                    quality: FilterQuality::Medium,
                },
            ]))),
            &theme,
        );

        assert_eq!(shortcut.effect.backdrop, full.effect.backdrop);
    }

    #[test]
    fn backdrop_and_effects_are_non_inherited_and_initial_clears_them() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let parent = ComputedStyle::compute(
            &initial,
            &StylePatch::new()
                .backdrop_style(BackdropStyle::blur(8.0))
                .effects(Arc::from([Effect::blur(3.0)])),
            &theme,
        );

        let child = ComputedStyle::compute(&parent, &StylePatch::new(), &theme);
        assert_eq!(child.effect, initial.effect);

        let mut reset = parent.clone();
        reset.apply(
            &parent,
            &StylePatch::new().no_backdrop().no_effects(),
            &theme,
        );
        assert_eq!(reset.effect, initial.effect);
    }

    #[test]
    fn effect_compute_resolves_tokens_and_normalizes_ranges() {
        let mut theme = Theme::default();
        theme.spacing_lg = 21.0;
        theme.radius_md = 7.0;
        theme.primary = Color::rgba(0.1, 0.2, 0.3, 0.4);
        let initial = ComputedStyle::initial(&theme);
        let style = BackdropStyle::new()
            .with_filters(Arc::from([
                BackdropFilter::Blur {
                    sigma_x: LengthValue::Spacing(SpacingToken::Lg),
                    sigma_y: LengthValue::Px(-2.0),
                    quality: FilterQuality::High,
                },
                BackdropFilter::Grayscale(2.0),
                BackdropFilter::HueRotate(std::f32::consts::TAU),
                BackdropFilter::Pixelate {
                    size: Size::new(
                        LengthValue::Px(-4.0),
                        LengthValue::Spacing(SpacingToken::Lg),
                    ),
                },
            ]))
            .opacity(4.0)
            .mask(BackdropMask::Shape {
                shape: MaskShape::RoundedRect(LengthValue::Radius(RadiusToken::Md)),
                transform: Affine::IDENTITY,
            });
        let computed = ComputedStyle::compute(
            &initial,
            &StylePatch::new()
                .backdrop_style(style)
                .effects(Arc::from([Effect::DropShadow {
                    color: ColorValue::Token(ColorToken::Primary),
                    offset_x: LengthValue::Spacing(SpacingToken::Lg),
                    offset_y: LengthValue::Px(3.0),
                    sigma_x: LengthValue::Px(-1.0),
                    sigma_y: LengthValue::Spacing(SpacingToken::Lg),
                    spread: LengthValue::Px(-5.0),
                    quality: FilterQuality::Low,
                }])),
            &theme,
        );

        let backdrop = computed.effect.backdrop.unwrap();
        assert_eq!(backdrop.opacity, 1.0);
        assert_eq!(
            backdrop.filters.as_ref(),
            [
                ComputedBackdropFilter::Blur {
                    sigma_x: 21.0,
                    sigma_y: 0.0,
                    quality: FilterQuality::High,
                },
                ComputedBackdropFilter::Grayscale(1.0),
                ComputedBackdropFilter::HueRotate(0.0),
                ComputedBackdropFilter::Pixelate {
                    size: Size::new(0.0, 21.0),
                },
            ]
        );
        assert_eq!(
            backdrop.mask,
            ComputedBackdropMask::Shape {
                shape: ComputedMaskShape::RoundedRect(7.0),
                transform: Affine::IDENTITY,
            }
        );
        assert!(matches!(
            &computed.effect.effects[0],
            ComputedEffect::DropShadow {
                color,
                offset: Point { x: 21.0, y: 3.0 },
                sigma_x: 0.0,
                sigma_y: 21.0,
                spread: 0.0,
                ..
            } if *color == theme.primary
        ));
    }

    #[test]
    fn effect_chain_order_is_preserved_by_state_hash_and_diff() {
        let matrix = [
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ];
        let first: Arc<[Effect]> = Arc::from([Effect::blur(2.0), Effect::ColorMatrix(matrix)]);
        let second: Arc<[Effect]> = Arc::from([Effect::ColorMatrix(matrix), Effect::blur(2.0)]);
        let first_backdrop = BackdropStyle::new().with_filters(Arc::from([
            BackdropFilter::Blur {
                sigma_x: LengthValue::Px(1.0),
                sigma_y: LengthValue::Px(1.0),
                quality: FilterQuality::Medium,
            },
            BackdropFilter::Brightness(0.8),
        ]));
        let second_backdrop = BackdropStyle::new().with_filters(Arc::from([
            BackdropFilter::Brightness(0.8),
            BackdropFilter::Blur {
                sigma_x: LengthValue::Px(1.0),
                sigma_y: LengthValue::Px(1.0),
                quality: FilterQuality::Medium,
            },
        ]));
        let style = Style::new()
            .backdrop_style(first_backdrop.clone())
            .effects(first.clone())
            .when(WidgetState::HOVERED, |patch| {
                patch
                    .backdrop_style(second_backdrop.clone())
                    .effects(second.clone())
            });
        let normal = style.patch_for_state(WidgetState::empty());
        let hovered = style.patch_for_state(WidgetState::HOVERED);
        assert_eq!(normal.effect.backdrop, StyleValue::Value(first_backdrop));
        assert_eq!(hovered.effect.backdrop, StyleValue::Value(second_backdrop));
        assert_eq!(normal.effect.effects, StyleValue::Value(first));
        assert_eq!(hovered.effect.effects, StyleValue::Value(second));

        let hash = |patch: &StylePatch| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            patch.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(hash(&normal), hash(&hovered));

        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let normal = ComputedStyle::compute(&initial, &normal, &theme);
        let hovered = ComputedStyle::compute(&initial, &hovered, &theme);
        assert_eq!(normal.diff(&hovered), StyleDiffFlags::EFFECT);
    }

    #[test]
    fn computed_style_diff_marks_inherited_text_changes_as_subtree_style() {
        let theme = Theme::default();
        let current = ComputedStyle::initial(&theme);
        let mut next = current.clone();
        next.text.font_size += 1.0;

        let flags = current.diff(&next);

        assert!(flags.contains(StyleDiffFlags::TEXT));
        assert!(!flags.contains(StyleDiffFlags::PAINT));
    }

    #[test]
    fn inherited_text_properties_flow_from_parent() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.apply(
            &ComputedStyle::initial(&theme),
            &StylePatch::new().color(Color::BLUE_500).font_size(20.0),
            &theme,
        );

        let mut child = parent.inherited_from(&theme);
        child.apply(&parent, &StylePatch::new(), &theme);

        assert_eq!(child.text.color, Color::BLUE_500);
        assert_eq!(child.text.font_size, 20.0);
        assert_eq!(
            child.paint.background,
            ComputedColorStyle::Solid(Color::TRANSPARENT)
        );
        assert_eq!(child.layout.padding, EdgeInsets::zero());
    }

    #[test]
    fn initial_resets_inheritable_values() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.apply(
            &ComputedStyle::initial(&theme),
            &StylePatch::new().font_size(20.0),
            &theme,
        );

        let mut patch = StylePatch::new();
        patch.text.font_size = StyleValue::Initial;
        let mut child = parent.inherited_from(&theme);
        child.apply(&parent, &patch, &theme);

        assert_eq!(child.text.font_size, theme.font_size(FontSizeToken::Md));
    }

    #[test]
    fn tokens_resolve_from_theme() {
        let mut theme = Theme::default();
        theme.primary = Color::rgb(0.2, 0.3, 0.4);
        theme.spacing_lg = 18.0;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &StylePatch::new()
                .background(ColorToken::Primary)
                .gap(SpacingToken::Lg),
            &theme,
        );

        assert_eq!(
            computed.paint.background,
            ComputedColorStyle::Solid(Color::rgb(0.2, 0.3, 0.4))
        );
        assert_eq!(computed.layout.gap, 18.0);
    }

    #[test]
    fn gradient_color_styles_resolve_tokens() {
        let mut theme = Theme::default();
        theme.primary = Color::rgb(0.2, 0.3, 0.4);
        theme.background = Color::WHITE;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &StylePatch::new().background(ColorStyle::linear_gradient(
                Point::new(0.0, 0.0),
                Point::new(1.0, 1.0),
                ColorToken::Primary,
                ColorToken::Background,
            )),
            &theme,
        );

        assert_eq!(
            computed.paint.background,
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: Point::new(0.0, 0.0),
                end: Point::new(1.0, 1.0),
                from: Color::rgb(0.2, 0.3, 0.4),
                to: Color::WHITE,
            })
        );
    }

    #[test]
    fn shadow_style_resolves_from_theme_and_can_reset() {
        let mut theme = Theme::default();
        theme.primary = Color::rgba(0.1, 0.2, 0.3, 0.4);
        theme.spacing_sm = 10.0;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &StylePatch::new().box_shadow(
                ColorToken::Primary,
                Point::new(2.0, 4.0),
                SpacingToken::Sm,
                3.0,
            ),
            &theme,
        );

        let shadow = computed.paint.shadow.unwrap();
        assert_eq!(shadow.color, Color::rgba(0.1, 0.2, 0.3, 0.4));
        assert_eq!(shadow.offset, Point::new(2.0, 4.0));
        assert_eq!(shadow.blur, 10.0);
        assert_eq!(shadow.spread, 3.0);

        computed.apply(&initial, &StylePatch::new().no_shadow(), &theme);

        assert_eq!(computed.paint.shadow, None);
    }

    #[test]
    fn style_patch_for_state_uses_base_without_state_rules() {
        let style = Style::from_patch(
            StylePatch::default()
                .background(Color::BLACK)
                .color(Color::WHITE),
        );

        let patch = style.patch_for_state(WidgetState::empty());

        assert_eq!(
            patch.paint.background,
            StyleValue::Value(ColorStyle::from(Color::BLACK))
        );
        assert_eq!(
            patch.text.color,
            StyleValue::Value(ColorValue::from(Color::WHITE))
        );
    }

    #[test]
    fn style_patch_for_state_applies_hover_rule_over_base() {
        let style = Style::from_patch(StylePatch::default().color(Color::WHITE))
            .when(WidgetState::HOVERED, |s| s.color(Color::BLACK));

        let normal = style.patch_for_state(WidgetState::empty());
        let hovered = style.patch_for_state(WidgetState::HOVERED);

        assert_eq!(
            normal.text.color,
            StyleValue::Value(ColorValue::from(Color::WHITE))
        );
        assert_eq!(
            hovered.text.color,
            StyleValue::Value(ColorValue::from(Color::BLACK))
        );
    }

    #[test]
    fn widget_state_matcher_supports_required_and_forbidden_bits() {
        let matcher = WidgetStateMatcher::new(WidgetState::HOVERED, WidgetState::DISABLED);

        assert!(matcher.matches(WidgetState::HOVERED));
        assert!(!matcher.matches(WidgetState::HOVERED | WidgetState::DISABLED));
        assert!(!matcher.matches(WidgetState::PRESSED));
    }

    #[test]
    fn style_reports_state_changes_that_affect_matching_rules() {
        let style = Style::new().when(WidgetState::HOVERED, |s| s.color(Color::WHITE));

        assert!(style.affects_state_change(WidgetState::empty(), WidgetState::HOVERED));
        assert!(style.affects_state_change(WidgetState::HOVERED, WidgetState::empty()));
        assert!(!style.affects_state_change(WidgetState::empty(), WidgetState::PRESSED));
    }

    #[test]
    fn style_tracks_state_dependencies_from_rules() {
        let style = Style::new().when_state(
            WidgetStateMatcher::new(WidgetState::HOVERED, WidgetState::DISABLED),
            |s| s.color(Color::WHITE),
        );

        assert!(style.state_deps().contains(WidgetState::HOVERED));
        assert!(style.state_deps().contains(WidgetState::DISABLED));
        assert!(!style.state_deps().contains(WidgetState::PRESSED));
    }

    #[test]
    fn style_patch_cache_is_invalidated_by_mutating_base_style() {
        let mut style = Style::new().color(Color::WHITE);
        let first = style.patch_for_state(WidgetState::empty());
        assert_eq!(
            first.text.color,
            StyleValue::Value(ColorValue::from(Color::WHITE))
        );

        style.text.color = StyleValue::Value(ColorValue::from(Color::BLACK));
        let second = style.patch_for_state(WidgetState::empty());
        assert_eq!(
            second.text.color,
            StyleValue::Value(ColorValue::from(Color::BLACK))
        );
    }

    #[test]
    fn matching_state_rules_merge_without_resetting_each_other() {
        let style = Style::from_patch(StylePatch::default().background(Color::BLACK))
            .when(WidgetState::HOVERED, |s| s.color(Color::WHITE))
            .when(WidgetState::HOVERED, |s| s.border_radius(4.0));

        let patch = style.patch_for_state(WidgetState::HOVERED);

        assert_eq!(
            patch.paint.background,
            StyleValue::Value(ColorStyle::from(Color::BLACK))
        );
        assert_eq!(
            patch.text.color,
            StyleValue::Value(ColorValue::from(Color::WHITE))
        );
        assert_eq!(
            patch.paint.border_radius,
            StyleValue::Value(LengthValue::Px(4.0))
        );
    }

    #[test]
    fn style_owns_transition_configuration() {
        let transition = Transition::new(Duration::from_millis(180)).ease(crate::Easing::CubicOut);
        let style = Style::new().background(Color::BLACK).transition(transition);

        assert_eq!(style.transition_config(), Some(transition));
        assert_eq!(style.clone(), style);
        assert_eq!(style.clone().clear_transition().transition_config(), None);

        let mut left = DefaultHasher::new();
        style.hash(&mut left);
        let mut right = DefaultHasher::new();
        style.clone().hash(&mut right);
        assert_eq!(left.finish(), right.finish());
    }

    #[test]
    fn style_merge_overrides_only_with_configured_transition() {
        let first = Transition::new(Duration::from_millis(100));
        let second = Transition::new(Duration::from_millis(240));
        let mut style = Style::new().transition(first);

        style.merge(&Style::new().background(Color::BLACK));
        assert_eq!(style.transition_config(), Some(first));

        style.merge(&Style::new().transition(second));
        assert_eq!(style.transition_config(), Some(second));
    }

    #[test]
    fn transform_is_non_inherited_and_state_patches_merge_by_component() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let style = Style::new()
            .translate_x(3.0)
            .when(WidgetState::PRESSED, |patch| patch.translate_y(2.0));

        let normal = ComputedStyle::compute(
            &initial,
            &style.patch_for_state(WidgetState::empty()),
            &theme,
        );
        let pressed = ComputedStyle::compute(
            &initial,
            &style.patch_for_state(WidgetState::PRESSED),
            &theme,
        );
        let child = ComputedStyle::compute(&pressed, &StylePatch::new(), &theme);

        assert_eq!(normal.transform.translate, Point::new(3.0, 0.0));
        assert_eq!(pressed.transform.translate, Point::new(3.0, 2.0));
        assert_eq!(child.transform, TransformStyle::IDENTITY);
        assert_eq!(normal.diff(&pressed), StyleDiffFlags::TRANSFORM);
    }

    #[test]
    fn transform_affine_uses_normalized_origin() {
        let transform = TransformStyle::new()
            .uniform_scale(0.5)
            .translate(Point::new(0.0, 2.0));
        let affine = transform.to_affine(Size::new(100.0, 20.0));

        assert_eq!(
            affine.transform_point(Point::new(50.0, 10.0)),
            Point::new(50.0, 12.0)
        );
        assert_eq!(
            affine.transform_point(Point::new(0.0, 0.0)),
            Point::new(25.0, 7.0)
        );
    }
}
